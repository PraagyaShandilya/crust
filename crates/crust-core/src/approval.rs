use std::{collections::HashMap, sync::Arc};

use tokio::sync::{Mutex, oneshot};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct PendingApproval {
    pub id: String,
    pub session_id: String,
    pub tool_name: String,
    pub args: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approved,
    Rejected { reason: String },
}

#[derive(Debug, Default, Clone)]
pub struct ApprovalQueue {
    pending: Arc<Mutex<HashMap<String, PendingApprovalEntry>>>,
}

#[derive(Debug)]
struct PendingApprovalEntry {
    approval: PendingApproval,
    tx: Option<oneshot::Sender<ApprovalDecision>>,
}

impl ApprovalQueue {
    pub async fn enqueue(
        &self,
        session_id: String,
        tool_name: String,
        args: String,
    ) -> (PendingApproval, oneshot::Receiver<ApprovalDecision>) {
        let approval = PendingApproval {
            id: Uuid::new_v4().to_string(),
            session_id,
            tool_name,
            args,
        };
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(
            approval.id.clone(),
            PendingApprovalEntry {
                approval: approval.clone(),
                tx: Some(tx),
            },
        );
        (approval, rx)
    }

    pub async fn approve(&self, id: &str) -> bool {
        self.resolve(id, ApprovalDecision::Approved).await
    }

    pub async fn reject(&self, id: &str, reason: String) -> bool {
        self.resolve(id, ApprovalDecision::Rejected { reason })
            .await
    }

    pub async fn pending_for_session(&self, session_id: &str) -> Vec<PendingApproval> {
        self.pending
            .lock()
            .await
            .values()
            .filter(|entry| entry.approval.session_id == session_id)
            .map(|entry| entry.approval.clone())
            .collect()
    }

    async fn resolve(&self, id: &str, decision: ApprovalDecision) -> bool {
        let Some(mut entry) = self.pending.lock().await.remove(id) else {
            return false;
        };
        if let Some(tx) = entry.tx.take() {
            let _ = tx.send(decision);
        }
        true
    }
}
