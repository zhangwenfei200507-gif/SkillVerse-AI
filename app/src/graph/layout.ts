import ELK from 'elkjs/lib/elk-api.js'
import workerUrl from 'elkjs/lib/elk-worker.min.js?url'
import type { ELK as ElkInstance } from 'elkjs/lib/elk-api.js'

interface LayoutNode {
  id: string
  kind: 'core' | 'branch' | 'capability'
  width?: number
  height?: number
}

interface LayoutEdge {
  id: string
  source: string
  target: string
}

let engine: ElkInstance | undefined

export async function layoutGraph(nodes: LayoutNode[], edges: LayoutEdge[]) {
  engine ??= new ELK({ workerUrl })
  return engine.layout({
    id: 'skillverse',
    layoutOptions: {
      'elk.algorithm': 'layered',
      'elk.direction': 'RIGHT',
      'elk.edgeRouting': 'SPLINES',
      'elk.layered.spacing.nodeNodeBetweenLayers': '120',
      'elk.spacing.nodeNode': '55',
      'elk.padding': '[top=36,left=30,bottom=36,right=60]',
    },
    children: nodes.map((node) => ({
      id: node.id,
      width: node.width ?? (node.kind === 'core' ? 172 : node.kind === 'branch' ? 168 : 214),
      height: node.height ?? (node.kind === 'core' ? 82 : 66),
    })),
    edges: edges.map((edge) => ({ id: edge.id, sources: [edge.source], targets: [edge.target] })),
  })
}
