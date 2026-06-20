import { create } from "zustand";

export interface Me {
  username: string;
  generation: number;
  login_at: number;
  expires_at: number;
}

interface MeState {
  me: Me | null;
  setMe: (me: Me) => void;
  clearMe: () => void;
}

export const useMeStore = create<MeState>((set) => ({
  me: null,
  setMe: (me) => set({ me }),
  clearMe: () => set({ me: null }),
}));
