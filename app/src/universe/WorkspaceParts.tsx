import { BookOpen, Box, ChevronRight, GitBranch, Layers3, Plug, SquareTerminal } from 'lucide-react'
import { stateLabels, type CapabilityKind, type SpatialNode } from './model'

const icons = { skill: BookOpen, plugin: Box, mcp: Plug, repository: SquareTerminal }

export function TypeIcon({ kind, size = 17 }: { kind: CapabilityKind; size?: number }) {
  const Icon = icons[kind]
  return <Icon size={size} aria-hidden="true" />
}

export function CapabilityDirectory({ nodes, compact = false, label, prefix, selected, onSelect }: {
  nodes: SpatialNode[]
  compact?: boolean
  label: string
  prefix: string
  selected: string
  onSelect: (node: SpatialNode) => void
}) {
  return <div className={'workspace-directory' + (compact ? ' is-compact' : '')} aria-label={label}>
    {nodes.map(node => {
      const item = node.capability
      const parentDepth = node.kind === 'capability' ? 'capability' : node.kind === 'cluster' && !node.id.startsWith('function:') ? 'type' : 'category'
      return <button key={node.id} type="button" className="workspace-directory-row" data-kind={parentDepth} aria-label={prefix + ' ' + node.name}
        aria-pressed={selected === node.id} onClick={() => onSelect(node)}>
        <span className="directory-icon" style={{ color: node.color }}>
          {item ? <TypeIcon kind={item.kind} /> : parentDepth === 'type' ? <Layers3 size={17} aria-hidden="true" /> : <GitBranch size={17} aria-hidden="true" />}
        </span>
        <span className="directory-copy">
          <strong>{node.name}</strong>
          {(!compact || item) && <small>{item ? [stateLabels[item.installation], item.source].join(' / ') : node.subtitle}</small>}
        </span>
        {compact && !item && <span className="directory-count">{node.count}</span>}
        <ChevronRight size={15} aria-hidden="true" />
      </button>
    })}
  </div>
}
