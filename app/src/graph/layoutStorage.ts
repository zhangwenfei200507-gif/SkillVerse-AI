export interface Point {
  x: number
  y: number
}

export type PositionMap = Record<string, Point>

const STORAGE_KEY = 'skillverse.graph-layout.v1'

export function getBrowserLayoutStorage(): Storage | undefined {
  try {
    return window.localStorage
  } catch {
    return undefined
  }
}

export function readLayoutPositions(
  storage: Pick<Storage, 'getItem'>,
  allowedIds: ReadonlySet<string>,
): PositionMap {
  try {
    const raw = storage.getItem(STORAGE_KEY)
    if (!raw) return {}
    const parsed: unknown = JSON.parse(raw)
    if (!parsed || typeof parsed !== 'object' || !('positions' in parsed)) return {}
    const positions = (parsed as { positions?: unknown }).positions
    if (!positions || typeof positions !== 'object' || Array.isArray(positions)) return {}

    return Object.fromEntries(Object.entries(positions).flatMap(([id, point]) => {
      if (!allowedIds.has(id) || !point || typeof point !== 'object') return []
      const candidate = point as Partial<Point>
      if (!Number.isFinite(candidate.x) || !Number.isFinite(candidate.y)) return []
      return [[id, { x: candidate.x as number, y: candidate.y as number }]]
    }))
  } catch {
    return {}
  }
}

export function writeLayoutPositions(storage: Pick<Storage, 'setItem'>, positions: PositionMap): boolean {
  try {
    storage.setItem(STORAGE_KEY, JSON.stringify({ version: 1, positions }))
    return true
  } catch {
    return false
  }
}

export function clearLayoutPositions(storage: Pick<Storage, 'removeItem'>): boolean {
  try {
    storage.removeItem(STORAGE_KEY)
    return true
  } catch {
    return false
  }
}
