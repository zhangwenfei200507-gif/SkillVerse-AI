import { Orbit } from 'lucide-react'
import type { SpatialNode } from './model'

export default function UniverseFallback({ nodes, selectedId, onSelect, message = '3D rendering is unavailable. Your capability data is accessible below.' }: {
  nodes: SpatialNode[]
  selectedId?: string
  onSelect: (node: SpatialNode) => void
  message?: string
}) {
  return <div className="universe-fallback" data-testid="universe-fallback">
    <p><Orbit size={16} aria-hidden="true" />{message}</p>
    <div>{nodes.map(node => <button key={node.id} aria-pressed={selectedId === node.id} onClick={() => onSelect(node)}>{node.name}</button>)}</div>
  </div>
}
