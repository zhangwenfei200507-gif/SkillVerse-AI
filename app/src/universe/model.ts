import type { PhysicalCapabilityObservation } from '../data/capabilityIdentity'
import { discoveryCategories, type CatalogItem } from '../domain/catalog'
import type { CodexScanReport, StoredLearningRecord } from '../data/desktopDatabase'

export type ScenePage = 'World Tree' | 'Discover' | 'Installed' | 'Learning' | 'Settings'
export const domains = ['Engineering', 'Knowledge', 'Automation', 'Frontend', 'Unclassified'] as const
export type Domain = typeof domains[number]
export type CapabilityKind = 'skill' | 'mcp' | 'plugin' | 'repository'
export type Installation = 'installed' | 'configured' | 'needs-review' | 'disabled' | 'candidate'
export interface Capability {
  id: string
  entityId: string
  name: string
  summary: string
  kind: CapabilityKind
  origin: 'local' | 'discovery'
  source: string
  sourceUrl?: string
  installation: Installation
  primaryCategory: Domain
  relatedCategories: Domain[]
  classification: 'catalog' | 'inferred' | 'unclassified' | 'manual' | 'rules'
  functionCategory: string
  functionCategories?: string[]
  learningStatus: StoredLearningRecord['status']
  item?: CatalogItem
  logicalId?: string
  physicalInstallations?: PhysicalCapabilityObservation[]
}
export interface SpatialNode {
  id: string
  name: string
  subtitle: string
  kind: 'core' | 'domain' | 'cluster' | 'capability' | 'stage'
  position: [number, number, number]
  color: string
  parentId?: string
  capability?: Capability
  count?: number
}
export const domainColors: Record<Domain, string> = {
  Engineering: '#6cabff', Knowledge: '#b6a1ff', Automation: '#f0d4a3', Frontend: '#72e2ec', Unclassified: '#9eafcc',
}
export const stateLabels: Record<Installation, string> = {
  installed: 'Found locally', configured: 'MCP configured · not connected', 'needs-review': 'Manifest needs review', disabled: 'Disabled', candidate: 'Discovery candidate',
}
export const typeLabels: Record<CapabilityKind, string> = { skill: 'Skills', mcp: 'MCP Servers', plugin: 'Plugins', repository: 'Tools / repositories' }
export const capabilityKinds: CapabilityKind[] = ['skill', 'mcp', 'plugin', 'repository']
export const functionLabels: Record<string, string> = { ...Object.fromEntries(discoveryCategories.map(({ id, label }) => [id, label])), unclassified: 'Pending classification' }
const functionRules: Array<[string, RegExp]> = [
  ['mobile', /\b(ios|android|swiftui|mobile|react.native|macos|expo)\b/i],
  ['security', /\b(security|audit|vulnerab|threat|pentest|sast)\b/i],
  ['databases', /\b(sql|sqlite|database|postgres|mongodb|supabase)\b/i],
  ['testing', /\b(testing|debug|tdd|unit.test|qa)\b/i],
  ['analytics', /\b(analytics|data.analysis|data.visualization|charts?|dashboard|statistics)\b/i],
  ['marketing', /\b(marketing|seo|social.media|campaign)\b/i],
  ['writing', /\b(writing|copywriting|documentation|changelog)\b/i],
  ['productivity', /\b(meeting|calendar|gmail|slack|productivity|clickup)\b/i],
  ['ai', /\b(llm|rag|fine.tun|embedding|machine.learning|agents.sdk)\b/i],
  ['presentations', /\b(pptx?|powerpoint|presentation|slides?)\b/i], ['video', /\b(video|animation|manim|remotion|hyperframes)\b/i],
  ['spreadsheets', /\b(spreadsheet|excel|xlsx?|csv)\b/i], ['documents', /\b(pdf|documents?|docx|word processing|markdown)\b/i],
  ['frontend', /\b(frontend|front-end|react|css|tailwind|web interface|ui design|figma)\b/i],
  ['backend', /\b(backend|back-end|sql|sqlite|database|rest api|server development|fastapi|django|express\.js)\b/i],
  ['browser', /\b(browser|playwright|selenium|web automation)\b/i], ['cloud', /\b(azure|cloud|deploy|kubernetes|docker|foundry)\b/i],
  ['development', /\b(code|coding|git|github|tdd|debug|test|developer|software|rust|python)\b/i],
  ['research', /\b(research|knowledge|search|context7|learn|reference)\b/i], ['images', /\b(image|photo|illustration|graphics|imagegen)\b/i], ['video', /\b(audio|speech|music|voice)\b/i],
]
export function classifyFunction(name: string, summary: string): string {
  return functionRules.find(([, rule]) => rule.test(name))?.[0] ?? functionRules.find(([, rule]) => rule.test(summary))?.[0] ?? 'unclassified'
}
const rules: Array<[Domain, RegExp]> = [
  ['Frontend', /\b(frontend|react|ui |user interface|css|design|figma|web design|tailwind)\b/i],
  ['Automation', /\b(automat|browser|playwright|workflow|agent-browser|selenium|task sched)/i],
  ['Knowledge', /\b(document|research|pdf|knowledge|learn|search|spreadsheet|presentation|notion|book|context7)\b/i],
  ['Engineering', /\b(code|coding|engineer|develop|git|debug|test|software|deploy|azure|rust|python|api|vercel)\b/i],
]
export function classify(name: string, summary: string): { primaryCategory: Domain; relatedCategories: Domain[]; classification: 'inferred' | 'unclassified' } {
  const categories = rules.filter(([, pattern]) => pattern.test(name + ' ' + summary)).map(([domain]) => domain)
  return { primaryCategory: categories[0] ?? 'Unclassified', relatedCategories: categories.slice(1), classification: categories.length ? 'inferred' : 'unclassified' }
}
export function buildCapabilities(items: CatalogItem[], scan: CodexScanReport | null): Capability[] {
  const local: Capability[] = []
  const used = new Map<string, number>()
  const add = (entityId: string, name: string, summary: string, kind: CapabilityKind, source: string, installation: Installation) => {
    const key = 'local:' + kind + ':' + entityId
    const duplicate = used.get(key) ?? 0
    used.set(key, duplicate + 1)
    local.push({ id: key + (duplicate ? ':' + duplicate : ''), entityId, name, summary, kind, origin: 'local', source, installation, ...classify(name, summary), functionCategory: classifyFunction(name, summary), learningStatus: 'NOT_LEARNED' })
  }
  scan?.skills.forEach((item) => add(item.id, item.name, item.description, 'skill', item.scope, !item.valid ? 'needs-review' : item.enabled === false ? 'disabled' : 'installed'))
  scan?.plugins.forEach((item) => add(item.id, item.name, item.version ? 'Plugin version ' + item.version : 'Detected local plugin metadata.', 'plugin', 'Codex plugin metadata', !item.valid ? 'needs-review' : item.enabled === false ? 'disabled' : 'installed'))
  scan?.mcpServers.forEach((item) => add(item.id, item.name, item.transport + ' transport. Configuration was found; the server was not started or runtime verified.', 'mcp', item.scope, item.enabled ? 'configured' : 'disabled'))
  const observations=new Map(scan?.identity?.observations.map(observation=>[observation.entityId,observation])??[])
  const groups=new Map<string,Capability[]>()
  for(const capability of local){const observation=observations.get(capability.entityId);const key=observation?.logicalId??capability.id;groups.set(key,[...(groups.get(key)??[]),capability])}
  const projected=[...groups.entries()].map(([logicalId,members])=>{
    const candidates=members.map(capability=>({capability,observation:observations.get(capability.entityId)}))
    const preferred=candidates.find(value=>value.observation?.evidence.sourceManifestMatchesCache===true)??candidates[0]
    const observation=preferred.observation
    if(!observation)return preferred.capability
    const current=candidates.flatMap(value=>value.observation?[value.observation]:[])
    const physical=new Map<string,PhysicalCapabilityObservation>()
    for(const item of [...(scan?.identity?.history??[]),...current]){if(item.logicalId===logicalId)physical.set(item.physicalId,item)}
    const history=[...physical.values()].filter(value=>!value.present)
    const versions=[...new Set(current.map(value=>value.packageVersion??'manifest '+value.manifestHash.slice(0,12)))]
    const context=members.length>1?' Current visible installations: '+members.length+'. Versions: '+versions.join(', ')+'. Active runtime version is unverified.':''
    const previous=history.length?' Historical physical installations: '+history.length+' (not installed); versions: '+[...new Set(history.map(value=>value.packageVersion??value.cacheVersion??'manifest revision'))].join(', ')+'.':''
    return {...preferred.capability,id:'local:'+preferred.capability.kind+':'+logicalId,logicalId,physicalInstallations:[...physical.values()],summary:preferred.capability.summary+context+previous+(previous||context?' Runtime remains unverified.':'')}
  })
  return [...projected, ...items.map((item): Capability => ({
    id: 'candidate:' + item.id, entityId: item.id, name: item.name, summary: item.summary, kind: item.kind ?? 'repository',
    origin: 'discovery', source: item.owner + '/' + item.repo, sourceUrl: item.githubUrl, installation: item.managedInstallation ? (item.managedInstallation.observedState ?? item.managedInstallation.state) === 'FILES_INSTALLED' ? 'installed' : (item.managedInstallation.observedState ?? item.managedInstallation.state) === 'CONFIGURED' ? 'configured' : 'needs-review' : 'candidate', primaryCategory: item.category,
    relatedCategories: [], classification: 'catalog', functionCategory: item.functionCategories?.[0] ?? classifyFunction(item.name, item.summary), functionCategories: item.functionCategories, learningStatus: 'NOT_LEARNED', item,
  }))]
}
export function matches(capability: Capability, query: string) {
  return (capability.name + ' ' + capability.summary + ' ' + capability.source + ' ' + capability.primaryCategory + ' ' + capability.kind + ' ' + (capability.functionCategories ?? [capability.functionCategory]).map((key) => functionLabels[key]).join(' ')).toLowerCase().includes(query.toLowerCase().trim())
}
export const belongsToFunction = (capability: Capability, key: string) => (capability.functionCategories ?? [capability.functionCategory]).includes(key)
export type Point3 = [number, number, number]
export type Positions = Record<string, Point3>
export const stageNames = ['Explore', 'Configure', 'Practice', 'Verify', 'Master'] as const
export function buildWorldNodes(capabilities: Capability[], expanded: Set<string>, openTypes: Set<string>, query: string): SpatialNode[] {
  const searching = Boolean(query.trim())
  const membersByFunction = new Map<string, Capability[]>()
  const firstPrimaryByFunction = new Map<string, Domain>()
  const visibleByFunction = new Map<string, Capability[]>()
  for (const item of capabilities) {
    if (!firstPrimaryByFunction.has(item.functionCategory)) firstPrimaryByFunction.set(item.functionCategory, item.primaryCategory)
    const visible = !searching || matches(item, query)
    for (const key of new Set(item.functionCategories ?? [item.functionCategory])) {
      const members = membersByFunction.get(key)
      if (members) members.push(item)
      else membersByFunction.set(key, [item])
      if (visible) {
        const matches = visibleByFunction.get(key)
        if (matches) matches.push(item)
        else visibleByFunction.set(key, [item])
      }
    }
  }
  const keys = Object.keys(functionLabels).filter((key) => membersByFunction.has(key))
  const activeFunction = [...openTypes][0]?.split(':')[1]
  const overview = expanded.size > 1 || (searching && !activeFunction)
  const half = Math.ceil(keys.length / 2)
  const nodes: SpatialNode[] = [{ id: 'codex-root', name: 'MY CODEX', subtitle: 'Capability universe', kind: 'core', position: [0, 0, 0], color: '#7fc9ff' }]
  for (const side of [-1, 1]) {
    const sideKeys = side < 0 ? keys.slice(0, half) : keys.slice(half)
    const sections = sideKeys.map((key) => {
      const members = visibleByFunction.get(key) ?? []
      const isOpen = expanded.has(key) || (searching && !activeFunction && members.length > 0)
      const typedMembers = new Map<CapabilityKind, Capability[]>()
      for (const item of members) {
        const typed = typedMembers.get(item.kind)
        if (typed) typed.push(item)
        else typedMembers.set(item.kind, [item])
      }
      const kinds = capabilityKinds.filter((kind) => typedMembers.has(kind))
      return { key, members, isOpen, kinds, typedMembers, span: overview && isOpen ? Math.max(1, kinds.length) : 1 }
    })
    const step = overview ? .78 : .94
    const totalRows = sections.reduce((sum, section) => sum + section.span, 0)
    let cursor = (totalRows - 1) * step / 2
    for (const section of sections) {
      const { key, members, kinds, typedMembers, isOpen, span } = section
      const y = cursor - (span - 1) * step / 2
      const color = domainColors[firstPrimaryByFunction.get(key) ?? 'Unclassified']
      const id = 'domain:' + key
      nodes.push({ id, name: functionLabels[key], subtitle: members.length + ' capabilities', count: members.length, kind: 'domain', position: [side * 3.6, y, 0], color, parentId: 'codex-root' })
      if (isOpen) kinds.forEach((kind, index) => {
        const typed = typedMembers.get(kind) ?? []
        const typeId = 'type:' + key + ':' + kind
        const localCount = typed.filter((item) => item.origin === 'local').length
        const centerY = Math.max(-3.4, Math.min(3.4, y))
        const typeY = overview ? cursor - index * step : centerY + ((kinds.length - 1) / 2 - index) * 1.05
        nodes.push({ id: typeId, name: typeLabels[kind], subtitle: localCount + ' local · ' + (typed.length - localCount) + ' candidates', count: typed.length, kind: 'cluster', position: [side * 7.2, typeY, 0], color, parentId: id })
        if (!openTypes.has(typeId)) return
        typed.forEach((capability, index) => nodes.push({
          id: capability.id, name: capability.name, subtitle: capability.installation !== 'candidate' ? stateLabels[capability.installation] : 'Discovery candidate',
          kind: 'capability', capability, position: [side * 10.8, (typed.length - 1) * .45 - index * .9, 0], color, parentId: typeId,
        }))
      })
      cursor -= span * step
    }
  }
  return nodes
}
export function orbitalNodes(capabilities: Capability[], center: Point3 = [-2.8, 0, 0]): SpatialNode[] {
  return capabilities.map((item, index) => {
    const ring = capabilities.length > 16 ? Math.floor(index / 12) : 0
    const ringCount = capabilities.length > 16 ? Math.min(12, capabilities.length - ring * 12) : capabilities.length
    const theta = Math.PI * .5 + (index % (capabilities.length > 16 ? 12 : Math.max(capabilities.length, 1))) / Math.max(ringCount, 1) * Math.PI * 2 + ring * .23
    const radius = (capabilities.length > 8 ? 5.3 : 3.9) + ring * 1.8
    return { id: item.id, name: item.name, subtitle: item.origin === 'local' ? stateLabels[item.installation] : item.kind.toUpperCase(), kind: 'capability', capability: item, color: domainColors[item.primaryCategory],
      position: [center[0] + Math.cos(theta) * radius, center[1] + Math.sin(theta) * radius * .72, Math.sin(theta * 2) * .65 - ring * .5], parentId: 'codex-root',
    }
  })
}
// The twenty-function grid has different hemispheres/rows from the old taxonomy.
// Preserve old profiles while starting this semantic grid from compatible slots.
export const positionStorageKey = (profile = 'folded') => 'skillverse.universe-layout.v7:' + profile
export function readPositions(profile = 'folded'): Positions {
  try { const raw = JSON.parse(localStorage.getItem(positionStorageKey(profile)) ?? '{}') as Positions
    const entries = Object.entries(raw).filter(([, p]) => Array.isArray(p) && p.length === 3 && p.every(Number.isFinite))
    // Reject coordinates from an older semantic grid, including an old state
    // retained by development hot reload. This tree only permits vertical node
    // movement, so a relocated core or incompatible column is never valid.
    if (entries.some(([id, p]) => {
      if (id === 'codex-root') return p.some((value) => Math.abs(value) > .001)
      const column = id.startsWith('domain:') ? 3.6 : id.startsWith('type:') ? 7.2 : id.startsWith('local:') || id.startsWith('candidate:') ? 10.8 : null
      return column !== null && (Math.abs(Math.abs(p[0]) - column) > .001 || Math.abs(p[2]) > .001)
    })) return {}
    return Object.fromEntries(entries)
  } catch { return {} }
}
export function storePositions(positions: Positions, profile = 'folded') {
  try { localStorage.setItem(positionStorageKey(profile), JSON.stringify(positions)); return true } catch { return false }
}
