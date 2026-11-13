const key = 'skillverse.first-run.v1'
export type WelcomeProgress = { step: number; dismissed: boolean }
export function readWelcome(): WelcomeProgress | null {
  try {
    const saved = JSON.parse(localStorage.getItem(key) ?? 'null')
    return saved && Number.isInteger(saved.step) && saved.step >= 0 && saved.step < 3
      ? { step: saved.step, dismissed: saved.dismissed === true } : null
  } catch { return null }
}
export function saveWelcome(step: number, dismissed: boolean) {
  try { localStorage.setItem(key, JSON.stringify({ step, dismissed })) } catch { /* Guide remains usable for this session. */ }
}
