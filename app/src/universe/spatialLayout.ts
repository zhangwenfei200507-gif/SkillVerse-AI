import type { Point3, Positions, SpatialNode } from './model'
export function nodeRadius(node: SpatialNode, columnar: boolean): number {
  return node.kind === 'core' ? (columnar ? 1.94 : 1.72) : node.kind === 'domain' ? (columnar ? .32 : .36) : node.kind === 'cluster' ? (columnar ? .25 : .34) : node.kind === 'stage' ? .62 : columnar ? .23 : .3
}
export interface SpatialConnection { id: string; from: Point3; to: Point3; color: string; targetId: string }
export function spatialConnections(nodes: SpatialNode[], positions: Positions, columnar: boolean): SpatialConnection[] {
  const byId = new Map(nodes.map(node => [node.id, node]))
  const positionOf = (node: SpatialNode) => positions[node.id] ?? node.position
  const ports = new Map<string, number>()
  if (columnar) {
    const groups = new Map<string, [SpatialNode[], SpatialNode[]]>()
    for (const node of nodes) {
      const parent = node.parentId ? byId.get(node.parentId) : undefined
      if (!parent) continue
      let sides = groups.get(parent.id)
      if (!sides) {
        sides = [[], []]
        groups.set(parent.id, sides)
      }
      sides[positionOf(node)[0] < positionOf(parent)[0] ? 0 : 1].push(node)
    }
    for (const sides of groups.values()) for (const siblings of sides) {
      siblings.sort((a, b) => positionOf(b)[1] - positionOf(a)[1])
      siblings.forEach((node, index) => {
        ports.set(node.id, siblings.length === 1 ? 0 : .72 - index / (siblings.length - 1) * 1.44)
      })
    }
  }
  return nodes.flatMap((node) => {
    const parent = node.parentId ? byId.get(node.parentId) : undefined
    if (!parent) return []
    const from = positionOf(parent)
    const to = positionOf(node)
    const parentRadius = nodeRadius(parent, columnar)
    const targetRadius = nodeRadius(node, columnar)
    if (columnar) {
      const direction = to[0] < from[0] ? -1 : 1
      const angle = ports.get(node.id) ?? 0
      return [{ id: parent.id + '/' + node.id, from: [from[0] + direction * Math.cos(angle) * parentRadius, from[1] + Math.sin(angle) * parentRadius, from[2]], to: [to[0] - direction * targetRadius, to[1], to[2]], color: node.color, targetId: node.id }]
    }
    const distance = Math.hypot(to[0] - from[0], to[1] - from[1], to[2] - from[2]) || 1
    const direction = to.map((value, index) => (value - from[index]) / distance)
    return [{ id: parent.id + '/' + node.id, from: from.map((value, index) => value + direction[index] * parentRadius) as Point3, to: to.map((value, index) => value - direction[index] * targetRadius) as Point3, color: node.color, targetId: node.id }]
  })
}
export function constrainColumnMove(nodes: SpatialNode[], positions: Positions, id: string, desired: Point3): Point3 {
  const node = nodes.find((entry) => entry.id === id)
  if (!node || node.kind === 'core') return node?.position ?? desired
  const siblings = nodes.filter((entry) => entry.kind === node.kind && Math.abs(entry.position[0] - node.position[0]) < .01).sort((a, b) => b.position[1] - a.position[1])
  const index = siblings.findIndex((entry) => entry.id === id)
  const gap = Math.max(.72, nodeRadius(node, true) * 2 + .16)
  const upper = index > 0 ? (positions[siblings[index - 1].id] ?? siblings[index - 1].position)[1] - gap : Math.max(7, node.position[1] + 1.5)
  const lower = index < siblings.length - 1 ? (positions[siblings[index + 1].id] ?? siblings[index + 1].position)[1] + gap : Math.min(-7, node.position[1] - 1.5)
  return [node.position[0], Math.max(lower, Math.min(upper, desired[1])), node.position[2]]
}
