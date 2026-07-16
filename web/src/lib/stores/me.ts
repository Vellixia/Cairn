import { create } from "zustand";
import type { Me } from "@/lib/api";

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
