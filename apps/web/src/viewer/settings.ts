// The viewer's settings (design proposal §3.1, §3.2): the viewer format's longest side and the
// frame cache's budget. Remembered per browser in `localStorage`, which can be missing or throw
// (private windows, blocked site data), so every access is guarded and the defaults stand in.
import { create } from 'zustand'

/** Longest sides the viewer format offers; `full` is every cell (the stream's largest size). */
export const MAX_SIZES = [256, 512, 1024, 2048, 'full'] as const
export type MaxSize = (typeof MAX_SIZES)[number]

/** The `maxSize` sent for `full`: the server's largest stream side, so the box factor is 1. */
export const FULL_SIZE = 16384

/** Cache budgets offered, in MiB. */
export const BUDGETS_MIB = [128, 256, 512, 1024, 2048, 4096] as const

export interface ViewerSettings {
  maxSize: MaxSize
  /** Bytes of decoded frames the cache may hold. */
  budget: number
}

export const DEFAULT_SETTINGS: ViewerSettings = { maxSize: 1024, budget: 512 * 2 ** 20 }

const KEY = 'cfd-flux.viewer.settings'

/** The `maxSize` query value of a setting. */
export function maxSizeParam(m: MaxSize): number {
  return m === 'full' ? FULL_SIZE : m
}

/** The settings stored in `storage`, each value checked; the defaults for anything else. */
export function loadSettings(storage: Pick<Storage, 'getItem'> | null): ViewerSettings {
  try {
    const raw = storage?.getItem(KEY)
    if (!raw) return DEFAULT_SETTINGS
    const v = JSON.parse(raw) as Partial<Record<keyof ViewerSettings, unknown>>
    return {
      maxSize: MAX_SIZES.includes(v.maxSize as MaxSize)
        ? (v.maxSize as MaxSize)
        : DEFAULT_SETTINGS.maxSize,
      budget: BUDGETS_MIB.some((b) => b * 2 ** 20 === v.budget)
        ? (v.budget as number)
        : DEFAULT_SETTINGS.budget,
    }
  } catch {
    return DEFAULT_SETTINGS
  }
}

/** Stores the settings; a storage that refuses (quota, privacy) only means they are not kept. */
export function saveSettings(storage: Pick<Storage, 'setItem'> | null, s: ViewerSettings): void {
  try {
    storage?.setItem(KEY, JSON.stringify(s))
  } catch {
    // not remembered: the session still uses them
  }
}

/** `window.localStorage`, or null where reading the property itself throws. */
function browserStorage(): Storage | null {
  try {
    return typeof localStorage === 'undefined' ? null : localStorage
  } catch {
    return null
  }
}

interface SettingsState extends ViewerSettings {
  setMaxSize(maxSize: MaxSize): void
  setBudget(budget: number): void
}

export const useViewerSettings = create<SettingsState>()((set, get) => {
  const save = () =>
    saveSettings(browserStorage(), { maxSize: get().maxSize, budget: get().budget })
  return {
    ...loadSettings(browserStorage()),
    setMaxSize: (maxSize) => {
      set({ maxSize })
      save()
    },
    setBudget: (budget) => {
      set({ budget })
      save()
    },
  }
})
