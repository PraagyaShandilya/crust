import { create } from 'zustand';

export type ActiveView = 'chat' | 'settings' | 'spaces';

interface UiState {
  sidebarOpen: boolean;
  setSidebarOpen: (sidebarOpen: boolean) => void;
  activeView: ActiveView;
  setActiveView: (view: ActiveView) => void;
}

export const useUiStore = create<UiState>((set) => ({
  sidebarOpen: false,
  setSidebarOpen: (sidebarOpen) => set({ sidebarOpen }),
  activeView: 'chat',
  setActiveView: (activeView) => set({ activeView }),
}));
