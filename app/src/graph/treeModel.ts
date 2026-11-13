import type { CatalogItem, Category } from '../domain/catalog'

export const TREE_PAGE_SIZE = 6

export type TreeNodeKind = 'root' | 'domain' | 'group' | 'capability'

export interface CapabilityTreeNode {
  id: string
  name: string
  kind: TreeNodeKind
  parentId: string | null
  children: string[]
  category?: Category
  itemId?: string
  depth: number
}

export interface CapabilityTree {
  rootId: string
  nodes: Record<string, CapabilityTreeNode>
  orderedIds: string[]
  domainIds: string[]
}

export type ExpansionMode = 'single' | 'multi'

const slug = (value: string) => value.trim().toLocaleLowerCase().replace(/[^\p{L}\p{N}]+/gu, '-')

export function buildCapabilityTree(items: readonly CatalogItem[], domains: readonly Category[]): CapabilityTree {
  const rootId = 'codex-root'
  const nodes: Record<string, CapabilityTreeNode> = {
    [rootId]: { id: rootId, name: 'MY CODEX', kind: 'root', parentId: null, children: [], depth: 0 },
  }
  const domainIds = domains.map((category) => {
    const id = `branch-${category}`
    nodes[id] = { id, name: category, kind: 'domain', parentId: rootId, children: [], category, depth: 1 }
    nodes[rootId].children.push(id)
    return id
  })

  for (const item of items) {
    let parentId = domainIds[domains.indexOf(item.category)]
    let depth = 2
    for (const segment of item.treePath ?? []) {
      const id = `group-${slug(item.category)}-${(item.treePath ?? []).slice(0, depth - 2 + 1).map(slug).join('--')}`
      if (!nodes[id]) {
        nodes[id] = { id, name: segment, kind: 'group', parentId, children: [], category: item.category, depth }
        nodes[parentId].children.push(id)
      }
      parentId = id
      depth += 1
    }
    nodes[item.id] = { id: item.id, name: item.name, kind: 'capability', parentId, children: [], category: item.category, itemId: item.id, depth }
    nodes[parentId].children.push(item.id)
  }

  return { rootId, nodes, orderedIds: Object.keys(nodes), domainIds }
}

export function getTreePath(tree: CapabilityTree, nodeId: string): string[] {
  const path: string[] = []
  let current: CapabilityTreeNode | undefined = tree.nodes[nodeId]
  while (current) {
    path.unshift(current.id)
    current = current.parentId ? tree.nodes[current.parentId] : undefined
  }
  return path
}

export function getDescendantIds(tree: CapabilityTree, nodeId: string): string[] {
  const result: string[] = []
  const visit = (id: string) => {
    for (const childId of tree.nodes[id]?.children ?? []) {
      result.push(childId)
      visit(childId)
    }
  }
  visit(nodeId)
  return result
}

export function toggleTreeExpansion(tree: CapabilityTree, expandedIds: ReadonlySet<string>, nodeId: string, mode: ExpansionMode = 'single'): Set<string> {
  const next = new Set(expandedIds)
  const descendants = new Set(getDescendantIds(tree, nodeId))
  if (next.has(nodeId)) {
    return new Set([...next].filter((id) => id !== nodeId && !descendants.has(id)))
  }
  const node = tree.nodes[nodeId]
  if (node?.kind === 'domain' && mode === 'single') {
    for (const domainId of tree.domainIds) {
      if (domainId === nodeId) continue
      const remove = new Set([domainId, ...getDescendantIds(tree, domainId)])
      for (const id of remove) next.delete(id)
    }
  }
  next.add(nodeId)
  return next
}

export function countCapabilities(tree: CapabilityTree, nodeId: string): number {
  const node = tree.nodes[nodeId]
  if (!node) return 0
  if (node.kind === 'capability') return 1
  return node.children.reduce((count, childId) => count + countCapabilities(tree, childId), 0)
}

export function getVisibleTreeIds(
  tree: CapabilityTree,
  expandedIds: ReadonlySet<string>,
  childLimits: Readonly<Record<string, number>> = {},
  revealIds: ReadonlySet<string> = new Set(),
  visibleDomainIds: ReadonlySet<string> = new Set(tree.domainIds),
): Set<string> {
  const visible = new Set<string>([tree.rootId])
  const revealPathIds = new Set([...revealIds].flatMap((id) => getTreePath(tree, id)))

  const addChildren = (parentId: string) => {
    const children = tree.nodes[parentId]?.children ?? []
    const limit = Math.max(1, childLimits[parentId] ?? TREE_PAGE_SIZE)
    const selected = new Set(children.slice(0, limit))
    if (revealPathIds.size) for (const childId of children) if (revealPathIds.has(childId)) selected.add(childId)
    for (const childId of selected) {
      visible.add(childId)
      if (expandedIds.has(childId)) addChildren(childId)
    }
  }

  for (const domainId of tree.domainIds) {
    if (!visibleDomainIds.has(domainId)) continue
    visible.add(domainId)
    if (expandedIds.has(domainId)) addChildren(domainId)
  }
  return visible
}

export function findCatalogMatches(items: readonly CatalogItem[], query: string): CatalogItem[] {
  const normalized = query.trim().toLocaleLowerCase()
  if (!normalized) return []
  return items.filter((item) => [item.name, item.summary, item.category, ...(item.treePath ?? []), ...item.capabilities]
    .join(' ').toLocaleLowerCase().includes(normalized))
}

/** Stable tree coordinates; only ids returned by getVisibleTreeIds become React Flow nodes. */
export function layoutVisibleTree(
  tree: CapabilityTree,
  visibleIds: ReadonlySet<string>,
  expandedIds: ReadonlySet<string>,
  compact = false,
  compactWidth = 390,
): Record<string, { x: number; y: number }> {
  const positions: Record<string, { x: number; y: number }> = {}
  const coreSize = compact ? 148 : 188
  positions[tree.rootId] = compact ? { x: Math.max(12, (compactWidth - coreSize) / 2), y: 20 } : { x: 25, y: 165 }

  if (compact) {
    let y = 205
    const branchX = Math.max(16, (compactWidth - 190) / 2)
    const childX = Math.max(12, (compactWidth - 250) / 2)
    for (const domainId of tree.domainIds) {
      if (!visibleIds.has(domainId)) continue
      positions[domainId] = { x: branchX, y }
      y += 95
      if (expandedIds.has(domainId)) {
        const placeCompact = (parentId: string, depth = 0) => {
          const children = tree.nodes[parentId].children.filter((id) => visibleIds.has(id))
          for (const childId of children) {
            positions[childId] = { x: childX, y }
            y += 84
            if (expandedIds.has(childId)) placeCompact(childId, depth + 1)
          }
        }
        placeCompact(domainId)
        y += 70
      } else y += 34
    }
    return positions
  }

  tree.domainIds.forEach((domainId, index) => {
    if (visibleIds.has(domainId)) positions[domainId] = { x: 310, y: 45 + index * 132 }
  })

  const nodeHeight = 82
  const gap = 18
  const subtreeHeight = (id: string): number => {
    if (!expandedIds.has(id)) return nodeHeight
    const children = tree.nodes[id]?.children.filter((childId) => visibleIds.has(childId)) ?? []
    if (!children.length) return nodeHeight
    return Math.max(nodeHeight, children.reduce((height, childId) => height + subtreeHeight(childId), gap * (children.length - 1)))
  }
  const placeChildren = (parentId: string) => {
    const parent = tree.nodes[parentId]
    const children = parent.children.filter((childId) => visibleIds.has(childId))
    if (!expandedIds.has(parentId) || !children.length) return
    const parentPosition = positions[parentId]
    const totalHeight = children.reduce((height, childId) => height + subtreeHeight(childId), gap * (children.length - 1))
    let cursor = parentPosition.y + nodeHeight / 2 - totalHeight / 2
    for (const childId of children) {
      const height = subtreeHeight(childId)
      positions[childId] = { x: parentPosition.x + (parent.kind === 'root' ? 0 : 320), y: cursor + (height - nodeHeight) / 2 }
      cursor += height + gap
      placeChildren(childId)
    }
  }
  for (const domainId of tree.domainIds) placeChildren(domainId)
  return positions
}
