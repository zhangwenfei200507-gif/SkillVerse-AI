import type { SpatialNode } from './model'

export default function UniverseFallback({ nodes, onSelect, message = "3D rendering is unavailable. Your capability data is accessible below." }: { nodes: SpatialNode[]; onSelect: (node: SpatialNode) => void; message?: string }) {
  return <div className="universe-fallback" data-testid="universe-fallback"><p>{message}</p><div>{nodes.map((node) => <button key={node.id} onClick={() => onSelect(node)}>{node.name}</button>)}</div></div>
}
