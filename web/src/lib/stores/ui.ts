import { create } from "zustand";

interface UIState {
  commandOpen: boolean;
  shortcutsOpen: boolean;
  setCommandOpen: (v: boolean) => void;
  setShortcutsOpen: (v: boolean) => void;
  toggleCommand: () => void;
  toggleShortcuts: () => void;
}

export const useUIStore = create<UIState>((set) => ({
  commandOpen: false,
  shortcutsOpen: false,
  setCommandOpen: (v) => set({ commandOpen: v }),
  setShortcutsOpen: (v) => set({ shortcutsOpen: v }),
  toggleCommand: () => set((s) => ({ commandOpen: !s.commandOpen })),
  toggleShortcuts: () => set((s) => ({ shortcutsOpen: !s.shortcutsOpen })),
}));
