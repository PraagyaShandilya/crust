use std::{
    collections::{HashMap, HashSet},
    env,
    path::{Path, PathBuf},
};

const DEFAULT_SKILLS_DIR: &str = "skills";

#[derive(Debug, Clone)]
pub struct MarkdownSkill {
    pub name: String,
    pub description: String,
    pub allowed_tools: Vec<String>,
    pub when_to_use: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub user_invocable: bool,
    pub path: PathBuf,
    pub content: Option<String>,
}

impl MarkdownSkill {
    fn matches(&self, requested: &str) -> bool {
        self.name.eq_ignore_ascii_case(requested)
            || self
                .path
                .parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(requested))
    }
}

fn configured_skills_dir() -> PathBuf {
    env::var("CRUST_SKILLS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(DEFAULT_SKILLS_DIR)
        })
}

fn collect_skill_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }

    let entries = std::fs::read_dir(dir)
        .map_err(|err| format!("failed to read skills dir {}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| err.to_string())?;
        let file_type = entry.file_type().map_err(|err| err.to_string())?;
        if file_type.is_dir() {
            let skill_path = entry.path().join("SKILL.md");
            if skill_path.exists() {
                files.push(skill_path);
            }
        }
    }
    Ok(())
}

fn split_markdown_frontmatter(content: &str) -> (HashMap<String, String>, &str) {
    let Some(rest) = content.strip_prefix("---\n") else {
        return (HashMap::new(), content);
    };
    let Some(end) = rest.find("\n---\n") else {
        return (HashMap::new(), content);
    };

    let mut metadata = HashMap::new();
    for line in rest[..end].lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        metadata.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    (metadata, &rest[end + "\n---\n".len()..])
}

fn parse_frontmatter_list(value: Option<&String>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|item| item.trim().trim_matches('"').trim_matches('\''))
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_frontmatter_bool(value: Option<&String>, default: bool) -> bool {
    value
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "true" | "yes" | "1"
            )
        })
        .unwrap_or(default)
}

fn parse_markdown_skill(path: PathBuf, content: String, include_content: bool) -> MarkdownSkill {
    let (frontmatter, body) = split_markdown_frontmatter(&content);
    let fallback_name = path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("skill")
        .to_string();

    let name = frontmatter
        .get("name")
        .filter(|name| !name.trim().is_empty())
        .cloned()
        .or_else(|| {
            body.lines()
                .find_map(|line| line.trim().strip_prefix("# ").map(str::trim))
                .filter(|heading| !heading.is_empty())
                .map(str::to_string)
        })
        .unwrap_or(fallback_name);

    let description = frontmatter
        .get("description")
        .filter(|description| !description.trim().is_empty())
        .cloned()
        .or_else(|| {
            body.lines()
                .map(str::trim)
                .find(|line| {
                    !line.is_empty()
                        && !line.starts_with('#')
                        && !line.starts_with("```")
                        && !line.starts_with('-')
                })
                .map(str::to_string)
        })
        .unwrap_or_else(|| "No description".to_string());

    MarkdownSkill {
        name,
        description,
        allowed_tools: parse_frontmatter_list(frontmatter.get("allowed-tools")),
        when_to_use: frontmatter
            .get("when_to_use")
            .or_else(|| frontmatter.get("when-to-use"))
            .cloned(),
        model: frontmatter.get("model").cloned(),
        effort: frontmatter.get("effort").cloned(),
        user_invocable: parse_frontmatter_bool(frontmatter.get("user-invocable"), true),
        path,
        content: include_content.then_some(content),
    }
}

pub fn load_markdown_skills() -> Result<Vec<MarkdownSkill>, String> {
    let skills_dir = configured_skills_dir();
    let mut files = Vec::new();
    collect_skill_files(&skills_dir, &mut files)?;
    files.sort();

    let mut skills = Vec::new();
    let mut names = HashSet::new();
    for path in files {
        let content = std::fs::read_to_string(&path)
            .map_err(|err| format!("failed to read skill {}: {err}", path.display()))?;
        let skill = parse_markdown_skill(path, content, false);
        let normalized_name = skill.name.to_ascii_lowercase();
        if !names.insert(normalized_name) {
            return Err(format!("duplicate markdown skill `{}`", skill.name));
        }
        skills.push(skill);
    }
    Ok(skills)
}

pub fn load_markdown_skill(requested: &str) -> Result<MarkdownSkill, String> {
    let skills = load_markdown_skills()?;
    let metadata = skills
        .into_iter()
        .find(|skill| skill.matches(requested))
        .ok_or_else(|| format!("skill `{requested}` not found"))?;
    let content = std::fs::read_to_string(&metadata.path)
        .map_err(|err| format!("failed to read skill {}: {err}", metadata.path.display()))?;
    Ok(parse_markdown_skill(metadata.path, content, true))
}

pub fn format_markdown_skills(skills: &[MarkdownSkill]) -> String {
    if skills.is_empty() {
        return format!(
            "No markdown skills found in `{}`. Set CRUST_SKILLS_DIR to override.",
            configured_skills_dir().display()
        );
    }

    let mut lines = vec![format!("Loaded {} markdown skill(s):", skills.len())];
    for skill in skills {
        let tools = if skill.allowed_tools.is_empty() {
            "tools: default".to_string()
        } else {
            format!("tools: {}", skill.allowed_tools.join(", "))
        };
        lines.push(format!(
            "- {} -> {}\n  {}\n  {}",
            skill.name,
            skill.path.display(),
            skill.description,
            tools
        ));
    }
    lines.join("\n")
}

pub fn format_markdown_skill_detail(skill: &MarkdownSkill) -> String {
    format!(
        "{}\npath: {}\ndescription: {}\nallowed-tools: {}\nwhen_to_use: {}\nmodel: {}\neffort: {}\nuser-invocable: {}\n\n{}",
        skill.name,
        skill.path.display(),
        skill.description,
        if skill.allowed_tools.is_empty() {
            "default".to_string()
        } else {
            skill.allowed_tools.join(", ")
        },
        skill.when_to_use.as_deref().unwrap_or("none"),
        skill.model.as_deref().unwrap_or("default"),
        skill.effort.as_deref().unwrap_or("default"),
        skill.user_invocable,
        skill.content.as_deref().unwrap_or("")
    )
}

pub fn render_skill_prompt(skill: &MarkdownSkill, args: &str) -> String {
    let content = skill.content.as_deref().unwrap_or("");
    let rendered = content
        .replace("{{args}}", args)
        .replace("$ARGUMENTS", args)
        .replace("{{ARGUMENTS}}", args);
    let allowed_tools = if skill.allowed_tools.is_empty() {
        "default tool policy".to_string()
    } else {
        skill.allowed_tools.join(", ")
    };
    format!(
        "Run skill `{}` with args:\n{}\n\nAllowed tools for this skill: {}\n\n{}",
        skill.name,
        args.trim(),
        allowed_tools,
        rendered
    )
}

pub fn parse_skill_command(prompt: &str) -> Option<(&str, &str)> {
    let args = prompt.strip_prefix("/skill ")?.trim();
    let mut parts = args.splitn(2, char::is_whitespace);
    let name = parts.next()?.trim();
    let skill_args = parts.next().unwrap_or("").trim();
    if name.is_empty() {
        None
    } else {
        Some((name, skill_args))
    }
}
