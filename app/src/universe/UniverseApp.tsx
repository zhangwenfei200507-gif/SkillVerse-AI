import {invoke} from '@tauri-apps/api/core'
import {IntelligencePanel} from './IntelligencePanel'
import {LearningWorkbench} from './LearningWorkbench'
import type { InstallOperation } from '../data/installationManager'
import { useCallback, useDeferredValue, useEffect, useMemo, useRef, useState, lazy, Suspense } from 'react'
import { isTauri } from '@tauri-apps/api/core'
import { ArrowLeft, ArrowUpRight, BookOpen, Box, ChevronRight, Compass, Database, Expand, Eye, Focus, GitBranch, GraduationCap, Layers3, Maximize2, Minus, Orbit, Plus, RefreshCw, RotateCcw, Search, Settings2, ShieldCheck, X } from 'lucide-react'
import { catalog, discoveryCategories, discoveryVerifiedAt, type CatalogItem } from '../domain/catalog'
import { checkDesktopDatabase, createDesktopDatabaseBackup, loadDesktopCatalog, loadDesktopCodexScan, loadDesktopLearningProgress, saveDesktopLearningProgress, saveDesktopPracticeEvidence, scanDesktopCodex, type CodexScanReport, type StoredLearningRecord } from '../data/desktopDatabase'
import { belongsToFunction, buildCapabilities, buildWorldNodes, capabilityKinds, functionLabels, typeLabels, matches, orbitalNodes, readPositions, stageNames, stateLabels, storePositions, type Capability, type CapabilityKind, type Positions, type ScenePage, type SpatialNode } from './model'
import type { RenderMetrics, UniverseSettings } from './UniverseEngine'
import FirstRun, { type FirstRunTarget } from './FirstRun'
import { readWelcome } from './firstRunState'
import UniverseFallback from './UniverseFallback'
import ModelRegistryPanel from './ModelRegistryPanel'
import { constrainColumnMove } from './spatialLayout'
import './universe.css'
import './universe-atmosphere.css'
import { GitHubCapabilityControls, GitHubDiscoveryControls } from './GitHubControls'
import type { DiscoveryState } from '../data/githubDiscovery'
const MissionControl = lazy(() => import('./MissionControl'))
const UniverseEngine = lazy(() => import('./UniverseEngine'))
const pages: ScenePage[] = ['World Tree', 'Discover', 'Installed', 'Learning', 'Settings']
const pageIcons = [GitBranch, Compass, Box, GraduationCap, Settings2]
const sceneTitles: Partial<Record<ScenePage, { title: string; description: string }>> = {
  Discover: { title: 'Discover', description: 'Explore the capability constellations' },
  Installed: { title: 'Installed', description: 'Your local Codex ecosystem' },
  Learning: { title: 'Learning universe', description: 'Follow the path from exploration to mastery' },
  Settings: { title: 'Universe settings', description: 'Display, integration and local data' },
}
const defaults: UniverseSettings = { quality: 'balanced', density: .75, motion: .45, reducedMotion: typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches, restoreScan: true, autoScan: false, nebula: true, bloom: .4, stars: true, particles: true, rendering: 'auto' }
function loadSettings(): UniverseSettings {
  try {
    const raw = JSON.parse(localStorage.getItem('skillverse.universe-settings.v1') ?? '{}')
    return { ...defaults, quality: ['low', 'balanced', 'high'].includes(raw.quality) ? raw.quality : defaults.quality,
      density: typeof raw.density === 'number' ? Math.max(.15, Math.min(1.5, raw.density)) : defaults.density,
      motion: typeof raw.motion === 'number' ? Math.max(0, Math.min(1, raw.motion)) : defaults.motion,
      stars: raw.stars !== false, particles: raw.particles !== false,
      rendering: raw.rendering === '2d' || raw.rendering === '3d' ? raw.rendering : 'auto',
      nebula: typeof raw.nebula === 'boolean' ? raw.nebula : true,
      bloom: typeof raw.bloom === 'number' ? Math.max(0, Math.min(.8, raw.bloom)) : defaults.bloom,
      reducedMotion: typeof raw.reducedMotion === 'boolean' ? raw.reducedMotion : defaults.reducedMotion,
      restoreScan: typeof raw.restoreScan === 'boolean' ? raw.restoreScan : true, autoScan: raw.autoScan === true,
    }
  } catch { return defaults }
}
export default function UniverseApp() {
  const desktop = isTauri()
  const [page, setPage] = useState<ScenePage>('World Tree')
  const [missionOpen,setMissionOpen]=useState(false)
  const [items, setItems] = useState<CatalogItem[]>(desktop ? [] : catalog)
  const [githubState, setGithubState] = useState<DiscoveryState | null>(null)
  const [pendingOperations,setPendingOperations] = useState<InstallOperation[]>([])
  const reloadCatalog = useCallback(async () => {
    if (!desktop) return
    const [loaded, restoredScan] = await Promise.all([loadDesktopCatalog(), loadDesktopCodexScan()])
    if (loaded.warnings.length) throw new Error(loaded.warnings.join(' '))
    setItems(loaded.items); setGithubState(loaded.githubState); setScan(restoredScan); setPendingOperations(loaded.installationState.history.map(item=>item.operation).filter(op=>op.state==='INTERRUPTED'))
  }, [desktop])
  const [scan, setScan] = useState<CodexScanReport | null>(null)
  const [settings, setSettings] = useState(loadSettings)
  const [startup, setStartup] = useState<{ newProfile: boolean; codexCliOnPath: boolean } | null>(null)
  const [guideStep, setGuideStep] = useState<number | null>(null)
  const [automaticMode, setAutomaticMode] = useState<'3d' | 'low' | '2d'>('3d')
  const slowSamples = useRef(0)
  useEffect(() => {
    if (!desktop) return
    let alive = true
    void invoke<{ newProfile: boolean; codexCliOnPath: boolean }>('desktop_startup').then(status => {
      if (!alive || typeof status?.newProfile !== 'boolean') return
      setStartup(status)
      const saved = readWelcome()
      if (saved ? !saved.dismissed : status.newProfile) setGuideStep(saved?.step ?? 0)
    }).catch(() => {})
    return () => { alive = false }
  }, [desktop])
  const [query, setQuery] = useState('')
  const deferredQuery = useDeferredValue(query)
  const [searchOpen, setSearchOpen] = useState(false)
  const searchInput = useRef<HTMLInputElement>(null)
  const searchResults = useRef<HTMLDivElement>(null)
  const [expanded, setExpanded] = useState(new Set<string>())
  const [clusters, setClusters] = useState(new Set<string>())
  const [reviewedCategories,setReviewedCategories] = useState<Record<string,string>>({})
  useEffect(()=>{let active=true;const load=()=>{if(isTauri())void invoke<{overrides?:[string,string][]}>('capability_intelligence',{action:'state',args:{}}).then(result=>{if(active)setReviewedCategories(Object.fromEntries(result.overrides??[]))}).catch(()=>{})};load();window.addEventListener('skillverse:classification-reviewed',load);return()=>{active=false;window.removeEventListener('skillverse:classification-reviewed',load)}},[])
  const [allMode, setAllMode] = useState(false)
  const [focus, setFocus] = useState(false)
  const [details, setDetails] = useState(true)
  const [selectedId, setSelectedId] = useState('codex-root')
  const [selectedCapabilityId, setSelectedCapabilityId] = useState<string | null>(null)
  const layoutProfile = allMode ? 'overview' : expanded.size ? 'branch:' + [...expanded].sort().join(',') + ':' + [...clusters].sort().join(',') : 'folded'
  const [layouts, setLayouts] = useState<Record<string, Positions>>({})
  const positions = useMemo(() => layouts[layoutProfile] ?? readPositions(layoutProfile), [layouts, layoutProfile])
  const [layoutSnapshot, setLayoutSnapshot] = useState<Positions | null>(null)
  const [fitToken, setFitToken] = useState(0)
  const [pulseToken, setPulseToken] = useState(0)
  const [zoom, setZoom] = useState(1)
  const [busy, setBusy] = useState(false)
  const [scanning, setScanning] = useState(false)
  const [scanError, setScanError] = useState('')
  const [diagnostics, setDiagnostics] = useState<string[]>([])
  const [showDiagnostics, setShowDiagnostics] = useState(false)
  const [metrics, setMetrics] = useState<RenderMetrics | null>(null)
  const reportMetrics = useCallback((sample: RenderMetrics) => {
    setMetrics(sample)
    if (!startup || settings.rendering !== 'auto' || missionOpen || document.hidden) return
    if (sample.software) {
      setAutomaticMode('2d')
      return
    }
    if (settings.reducedMotion || settings.motion === 0 || sample.fps <= 0) return
    slowSamples.current = sample.fps < 24 && sample.p95 > 50 ? slowSamples.current + 1 : 0
    if (slowSamples.current >= 3) {
      slowSamples.current = 0
      setAutomaticMode(mode => mode === '3d' && settings.quality !== 'low' ? 'low' : '2d')
    }
  }, [startup, settings.rendering, settings.reducedMotion, settings.motion, settings.quality, missionOpen])
  const use2d = settings.rendering === '2d' || (settings.rendering === 'auto' && automaticMode === '2d')
  const reducedQuality = settings.rendering === 'auto' && automaticMode === 'low'
  const [vaultType, setVaultType] = useState<string | null>(null)
  const [functionCategory, setFunctionCategory] = useState<string | null>(null)
  const [discoverySort, setDiscoverySort] = useState('recommended')
  const [layoutNotice, setLayoutNotice] = useState('')
  const [stage, setStage] = useState(0)
  const [learning, setLearning] = useState<Record<string, StoredLearningRecord>>({})
  const [learningViews,setLearningViews]=useState<Record<string,string[]>>({})
  const publishLearningView=useCallback((id:string,labels:string[])=>setLearningViews(old=>JSON.stringify(old[id])===JSON.stringify(labels)?old:{...old,[id]:labels}),[])
  const [database, setDatabase] = useState(desktop ? 'Checking' : 'Browser preview')
  const [backup, setBackup] = useState('')
  const [settingsTab, setSettingsTab] = useState('Display')
  const initial = useRef<Promise<void> | null>(null)
  const reportError = useCallback((message: string) => setDiagnostics((current) => [...new Set([...current, message])]), [])
  useEffect(() => {
    if (!desktop) return
    let active = true
    initial.current ??= (async () => {
      const result = await Promise.allSettled([loadDesktopCatalog(), settings.autoScan ? scanDesktopCodex() : settings.restoreScan ? loadDesktopCodexScan() : Promise.resolve(null), checkDesktopDatabase()])
      const [catalogResult, scanResult, dbResult] = result
      if (catalogResult.status === 'fulfilled') { setItems(catalogResult.value.items); setGithubState(catalogResult.value.githubState); const pending=catalogResult.value.installationState.history.map(item=>item.operation).filter(op=>op.state==='INTERRUPTED'); setPendingOperations(pending); if(pending.length)setShowDiagnostics(true); catalogResult.value.warnings.forEach(reportError) }
      else reportError('The SQLite candidate catalog could not be loaded.')
      if (scanResult.status === 'fulfilled') setScan(scanResult.value)
      else reportError('The saved Codex scan could not be restored. Scan again to retry.')
      setDatabase(dbResult.status === 'fulfilled' ? 'SQLite ready' : 'Unavailable')
      if (dbResult.status === 'rejected') reportError('SQLite integrity check failed.')
    })()
    void initial.current.then(() => { if (active) setBusy(false) })
    return () => { active = false }
    // Startup preferences intentionally apply the next time the application opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [desktop, reportError])
  useEffect(() => { try { localStorage.setItem('skillverse.universe-settings.v1', JSON.stringify(settings)) } catch { reportError('Display preferences could not be saved.') } }, [settings, reportError])
  useEffect(() => {
    if (!focus) return
    const key = (event: KeyboardEvent) => { if (event.key === 'Escape') setFocus(false) }
    window.addEventListener('keydown', key)
    return () => window.removeEventListener('keydown', key)
  }, [focus])
  const capabilities = useMemo(() => buildCapabilities(items, scan).map((original) => {
    const category=reviewedCategories[original.logicalId??("logical:github:"+original.entityId)]
    const item=category&&original.classification!=="manual"?{...original,functionCategory:category,functionCategories:[category],classification:"manual" as const}:original
    const classification = githubState?.classifications.find((entry) => entry.entityId === item.entityId)
    return { ...item, learningStatus: learning[item.entityId]?.status ?? item.learningStatus,
      ...(classification ? { functionCategory: classification.primary ?? 'unclassified', functionCategories: classification.primary ? [classification.primary, ...classification.related] : ['unclassified'], classification: classification.manual ? 'manual' as const : classification.state === 'PENDING_REVIEW' ? 'unclassified' as const : 'rules' as const } : {}),
    }
  }), [items, scan, reviewedCategories, learning, githubState])
  const local = useMemo(() => capabilities.filter((item) => item.origin === 'local'), [capabilities])
  const candidates = useMemo(() => capabilities.filter((item) => item.origin === 'discovery'), [capabilities])
  const selectedCapability = capabilities.find((item) => item.id === selectedCapabilityId)
  const selectedEntityId = selectedCapability?.entityId
  useEffect(() => {
    if (!desktop || !selectedEntityId) return
    let active = true
    void loadDesktopLearningProgress(selectedEntityId).then((record) => { if (active && record) setLearning((old) => ({ ...old, [record.entityId]: record })) }).catch(() => reportError('Learning data could not be loaded.'))
    return () => { active = false }
  }, [desktop, selectedEntityId, reportError])
  const worldNodes = useMemo(() => buildWorldNodes(capabilities, expanded, clusters, deferredQuery), [capabilities, expanded, clusters, deferredQuery])
  const searchMatches = useMemo(() => page === 'World Tree' && searchOpen && deferredQuery.trim()
    ? capabilities.filter((item) => matches(item, deferredQuery))
    : [], [capabilities, deferredQuery, page, searchOpen])
  const searchVisible = page === 'World Tree' && searchOpen && Boolean(query.trim())
  const focusSearchResult = (index: number) => {
    searchResults.current?.querySelectorAll<HTMLButtonElement>('button')[index]?.focus()
  }
  const scenePool = useMemo(() => {
    const pool = (page === 'Discover' ? candidates : local).filter((item) => (!functionCategory || belongsToFunction(item, functionCategory)) && (!vaultType || item.kind === vaultType) && matches(item, query))
    if (page !== 'Discover' || discoverySort === 'recommended') return pool
    return pool.sort((a, b) => discoverySort === 'stars' ? (b.item?.repositoryStars ?? -1) - (a.item?.repositoryStars ?? -1) || a.name.localeCompare(b.name) : (b.item?.repositoryPushedAt ?? '').localeCompare(a.item?.repositoryPushedAt ?? '') || a.name.localeCompare(b.name))
  }, [page, candidates, local, functionCategory, vaultType, query, discoverySort])
  const collectionIsLeaf = Boolean(query.trim() || functionCategory && vaultType)
  const nodes = useMemo((): SpatialNode[] => {
    if (page === 'World Tree') return worldNodes
    const core: SpatialNode = { id: 'codex-root', name: page === 'Discover' ? 'DISCOVER' : page === 'Installed' ? 'LOCAL CODEX' : page === 'Settings' ? 'CONTROL CONSOLE' : 'LEARNING', subtitle: page === 'Discover' ? 'Capability constellation' : page === 'Installed' ? local.length + ' observed capabilities' : 'Your capability universe', kind: 'core', position: [-3.1, 0, 0], color: '#8bc6ff' }
    if (page === 'Discover' || page === 'Installed') {
      if (functionCategory) { core.name = vaultType ? typeLabels[vaultType as CapabilityKind] : functionLabels[functionCategory]; core.subtitle = vaultType ? functionLabels[functionCategory] : 'Choose capability type' }
      if (collectionIsLeaf) return [core, ...orbitalNodes(scenePool, core.position)]
      const groups = !functionCategory ? Object.keys(functionLabels).filter((key) => scenePool.some((item) => belongsToFunction(item, key))).map((key) => ({ id: 'function:' + key, name: functionLabels[key], count: scenePool.filter((item) => belongsToFunction(item, key)).length })) : capabilityKinds.filter((kind) => scenePool.some((item) => item.kind === kind)).map((kind) => ({ id: 'vault:' + kind, name: typeLabels[kind], count: scenePool.filter((item) => item.kind === kind).length }))
      return [core, ...groups.map((group, index): SpatialNode => {
        const half = Math.ceil(groups.length / 2)
        const side = index < half ? -1 : 1
        const row = index < half ? index : index - half
        const rows = side < 0 ? half : groups.length - half
        const theta = functionCategory
          ? groups.length === 1 ? 0 : Math.PI + index / (groups.length - 1) * Math.PI
          : Math.PI * .5 + index / Math.max(groups.length, 1) * Math.PI * 2
        const position: [number, number, number] = groups.length > 8 ? [core.position[0] + side * 4.8, ((rows - 1) / 2 - row) * 1.12, -.3] : [core.position[0] + Math.cos(theta) * 3.9, Math.sin(theta) * 2.8, Math.sin(theta * 2) * .6]
        return { ...group, kind: 'cluster', subtitle: group.count + ' capabilities', parentId: core.id, position, color: ['#7dbfff', '#b1a1e8', '#8fd6da'][index % 3] }
      })]
    }
    if (page === 'Learning') return stageNames.map((name, index): SpatialNode => ({ id: 'stage:' + index, kind: 'stage', name, subtitle: learningViews[selectedEntityId??'']?.[index]??(index === 0 && selectedCapability?.learningStatus === 'EXPLORING' ? 'Exploring' : index === 2 && selectedCapability?.learningStatus === 'SELF_REPORTED' ? 'Self reported' : index === 3 && selectedCapability?.learningStatus === 'VERIFIED' ? 'Evidence verified' : 'Not completed'), parentId: index ? 'stage:' + (index - 1) : undefined, color: ['#6fafff', '#a296ed', '#8ebcff', '#91dadf', '#dad6f6'][index], position: [[-6.6, -2.3, 1.2], [-4.25, -1.2, .4], [-1.9, .05, -.4], [.1, 1.5, -1.7], [1.5, 2.9, -3]][index] as [number, number, number] }))
    return [{ ...core, position: [-5.5, -.3, -2.5] }]
  }, [page, worldNodes, local, scenePool, vaultType, functionCategory, collectionIsLeaf, selectedCapability?.learningStatus, selectedEntityId, learningViews])
  const selectedNode = nodes.find((node) => node.id === selectedId)
  const changePage = (next: ScenePage) => { slowSamples.current = 0; setPage(next); setQuery(''); setSearchOpen(false); setVaultType(null); setFunctionCategory(null); setFocus(false); setDetails(true); setSelectedId('codex-root'); setZoom(1); setFitToken((v) => v + 1) }
  const locateCapability = (capability: Capability, keepQuery = false) => {
    setMissionOpen(false)
    if (!keepQuery) changePage('World Tree')
    else setSearchOpen(false)
    setSelectedCapabilityId(capability.id)
    setSelectedId(capability.id)
    setAllMode(false)
    setExpanded(new Set([capability.functionCategory]))
    setClusters(new Set(['type:' + capability.functionCategory + ':' + capability.kind]))
    setDetails(true)
    if (keepQuery) setFitToken((value) => value + 1)
  }
  const locateInstalled = (capability: Capability) => {
    changePage('Installed')
    setFunctionCategory(capability.functionCategory)
    setVaultType(capability.kind)
    setSelectedCapabilityId(capability.id)
    setSelectedId(capability.id)
  }
  const closeGuide = (target?: FirstRunTarget) => {
    setGuideStep(null)
    if (target === 'installed') changePage('Installed')
    if (target === 'providers') { changePage('Settings'); setSettingsTab('AI Providers') }
    if (target === 'mission') setMissionOpen(true)
    if (target === 'world') changePage('World Tree')
  }
  const select = useCallback((node: SpatialNode) => {
    setPulseToken((token) => token + 1)
    setSelectedId(node.id); setDetails(true)
    if (node.capability) setSelectedCapabilityId(node.capability.id)
    if (node.kind === 'domain') {
      const domain = node.id.slice(7)
      setAllMode(false); setClusters(new Set())
      setExpanded((old) => old.has(domain) && old.size === 1 ? new Set() : new Set([domain]))
    }
    if (node.id.startsWith('function:')) { setFunctionCategory(node.id.slice(9)); setVaultType(null);  }
    else if (node.id.startsWith('vault:')) { setVaultType(node.id.slice(6));  }
    else if (node.id.startsWith('type:')) { setAllMode(false); setExpanded(new Set([node.id.split(':')[1]])); setClusters((old) => old.has(node.id) ? new Set() : new Set([node.id])) }
    if (node.kind === 'stage') setStage(Number(node.id.slice(6)))
  }, [])
  const updatePositions = useCallback((next: Positions, persist: boolean) => {
    setLayouts((previous) => ({ ...previous, [layoutProfile]: next }))
    if (persist && !storePositions(next, layoutProfile)) {
      reportError('Spatial layout could not be saved. Changes are available for this session.')
      return false
    }
    return true
  }, [layoutProfile, reportError])
  const move = useCallback((id: string, position: [number, number, number], commit = false) => {
    const safe = constrainColumnMove(worldNodes, positions, id, position)
    updatePositions({ ...positions, [id]: safe }, commit)
    return safe
  }, [worldNodes, positions, updatePositions])
  useEffect(() => { setLayoutSnapshot(null); setLayoutNotice('') }, [layoutProfile])
  const resetLayout = () => {
    setLayoutSnapshot(positions)
    const next = { ...positions, ...Object.fromEntries(worldNodes.map((node) => [node.id, node.position])) }
    const saved = updatePositions(next, true)
    setZoom(1)
    setFitToken((value) => value + 1)
    setLayoutNotice(saved ? worldNodes.length + ' visible nodes repositioned and view fitted. Expanded branches are unchanged.' : 'Layout reset for this session. Preferences could not be saved.')
  }
  const scanAgain = async () => {
    if (!desktop || scanning) return
    setScanning(true)
    setScanError('')
    try { setScan(await scanDesktopCodex()) }
    catch {
      const message = 'Codex scan failed. The last saved result was kept.'
      setScanError(message)
      reportError(message)
    }
    finally { setScanning(false) }
  }
  const updateLearning = async (status: 'EXPLORING' | 'SELF_REPORTED') => {
    if (!desktop || !selectedCapability) return
    setBusy(true)
    try { const record = await saveDesktopLearningProgress(selectedCapability.entityId, status); setLearning((old) => ({ ...old, [record.entityId]: record })) }
    catch { reportError('Learning progress could not be saved. Existing progress was kept.') }
    finally { setBusy(false) }
  }
  const updateSettings = <K extends keyof UniverseSettings>(key: K, value: UniverseSettings[K]) => {
    if (key === 'rendering' || key === 'quality') {
      slowSamples.current = 0
      setAutomaticMode('3d')
    }
    setSettings((old) => ({ ...old, [key]: value }))
  }
  const sceneTitle = functionCategory ? undefined : sceneTitles[page]
  const allDiagnostics = [...new Set([...diagnostics, ...(scan?.warnings ?? []),...pendingOperations.map(op=>'Interrupted installation operation '+op.id+'. Review its source to recover safely.')])]
  return <main className={'universe-app ' + (focus ? 'is-focused' : '')} data-scene={page} data-motion={settings.reducedMotion || settings.motion === 0 ? 'reduced' : 'full'} data-mission-open={missionOpen} data-render-engine={use2d ? 'accessible-2d' : 'react-three-fiber'} data-render-mode={use2d ? '2d' : reducedQuality ? 'low' : '3d'} data-starfield="drei-stars-sparkles" data-active-stars={metrics?.stars ?? 0} data-active-sparkles={metrics?.sparkles ?? 0} data-render-fps={metrics?.fps ?? 0} data-frame-p95={metrics?.p95 ?? 0} data-gpu-geometries={metrics?.geometries ?? 0} data-gpu-textures={metrics?.textures ?? 0} data-render-calls={metrics?.calls ?? 0} data-render-triangles={metrics?.triangles ?? 0} data-webgl-version={metrics?.renderer ?? ''} data-local-count={local.length} data-discovery-count={candidates.length} data-visible-nodes={nodes.length} data-cosmic-layers={metrics?.cosmic?.layers ?? 0} data-nebula-technique={metrics?.cosmic?.nebulaTechnique ?? ""} data-nebula-steps={metrics?.cosmic?.steps ?? 0} data-luminous-stars={metrics?.cosmic?.luminousStars ?? 0} data-foreground-particles={metrics?.cosmic?.foreground ?? 0} data-star-depth-span={metrics?.cosmic?.depthSpan ?? 0} data-nebula-buffer={JSON.stringify(metrics?.cosmic?.bufferSize ?? [])} data-star-probes={JSON.stringify(metrics?.cosmic?.probes ?? [])} data-bloom-enabled={metrics?.bloom ?? false}>
    <div style={{display:'contents'}} inert={missionOpen}>{use2d ? <UniverseFallback nodes={nodes} onSelect={select} message="Lightweight 2D mode. Select any visible capability below."/> : <Suspense fallback={<div className="universe-loading">Opening your universe…</div>}><UniverseEngine page={page} nodes={nodes} selectedId={selectedId} positions={positions} settings={reducedQuality ? { ...settings, quality: 'low' } : settings} focus={focus} background={missionOpen} zoom={zoom} fitToken={fitToken} pulseToken={pulseToken} onSelect={select} onMove={move} onMetrics={reportMetrics} /></Suspense>}</div>
    {guideStep !== null && <FirstRun initialStep={guideStep} codexDetected={startup?.codexCliOnPath ?? null} onClose={closeGuide}/>}
    <div className="universe-vignette" aria-hidden="true" />
    {missionOpen&&<Suspense fallback={<div className="universe-loading">Opening Mission Control…</div>}><MissionControl reducedMotion={settings.reducedMotion||settings.motion===0} preferredCapability={selectedCapability && (selectedCapability.logicalId ?? 'logical:github:' + selectedCapability.entityId)} onClose={()=>setMissionOpen(false)} onLocate={id=>{const cap=capabilities.find(c=>(c.logicalId??('logical:github:'+c.entityId))===id);if(cap)locateCapability(cap)}} onLearn={id=>{const cap=capabilities.find(c=>(c.logicalId??('logical:github:'+c.entityId))===id);if(cap){setMissionOpen(false);setSelectedCapabilityId(cap.id);changePage('Learning')}}}/></Suspense>}
    {!focus && <header className="universe-nav"><button className="universe-brand" onClick={() => changePage('World Tree')} aria-label="SkillVerse home"><span><Orbit size={25} /></span><div><strong>SkillVerse</strong><small>Codex capability universe</small></div></button><nav aria-label="Main navigation">{pages.map((item, index) => { const Icon = pageIcons[index]; return <button key={item} aria-label={item} aria-current={!missionOpen && page === item ? 'page' : undefined} onClick={() => {setMissionOpen(false);changePage(item)}}><Icon size={17} /><span>{item}</span></button> })}<button aria-label="Mission Control" aria-current={missionOpen?'page':undefined} onClick={()=>setMissionOpen(true)}><GitBranch size={17}/><span>Mission Control</span></button></nav><span className="universe-connection"><i className={scan ? 'ready' : ''} />{desktop ? 'Local Codex' : 'Browser preview'}</span></header>}
    <div style={{display:'contents',visibility:missionOpen?'hidden':undefined}} inert={missionOpen} aria-hidden={missionOpen}>
    {!focus && <div className="spatial-toolbar"><div className="spatial-breadcrumb"><span>Universe</span><ChevronRight size={12} /><strong>{page}</strong>{(page === 'Discover' || page === 'Installed') && functionCategory && <><ChevronRight size={12} /><span>{functionLabels[functionCategory]}</span>{vaultType && <><ChevronRight size={12} /><strong>{typeLabels[vaultType as CapabilityKind]}</strong></>}</>}</div>{sceneTitle && <div className="scene-title"><h1>{sceneTitle.title}</h1><p>{sceneTitle.description}</p></div>}{page !== "Learning" && page !== "Settings" && <div className="spatial-tools"><label className="spatial-search"><Search size={16} /><input ref={searchInput} aria-label="Search capabilities" aria-controls={searchVisible ? 'capability-search-results' : undefined} placeholder="Find a capability…" value={query} onChange={(event) => { setQuery(event.target.value); setSearchOpen(true) }} onKeyDown={(event) => {
        if (page !== 'World Tree' || !query.trim()) return
        if (event.key === 'Escape') { event.preventDefault(); setSearchOpen(false) }
        else if (event.key === 'ArrowDown') {
          event.preventDefault()
          if (searchOpen) focusSearchResult(0)
          else { setSearchOpen(true); requestAnimationFrame(() => focusSearchResult(0)) }
        } else if (event.key === 'Enter' && searchMatches[0]) {
          event.preventDefault()
          locateCapability(searchMatches[0], true)
        }
      }} />{query && <button aria-label="Clear search" onClick={() => setQuery('')}><X size={13} /></button>}</label>
      {(page === 'Discover' || page === 'Installed') && functionCategory && <button className="vault-back" onClick={() => { if (vaultType) setVaultType(null); else setFunctionCategory(null); setSelectedId('codex-root'); setQuery('') }}><ArrowLeft size={13} />{vaultType ? 'Back to types' : 'All categories'}</button>}
      {page === 'Discover' && collectionIsLeaf && <label className="discovery-sort"><span>Order</span><select aria-label="Sort discoveries" value={discoverySort} onChange={(event) => setDiscoverySort(event.target.value)}><option value="recommended">Curated</option><option value="stars">Repository stars</option><option value="recent">Recently maintained</option></select></label>}
    </div>}</div>}
    {page === 'World Tree' && !focus && <div className="tree-actions"><button onClick={() => { setAllMode(!allMode); setExpanded(allMode ? new Set() : new Set(Object.keys(functionLabels))); setClusters(new Set()); setSelectedId('codex-root'); setFitToken((v) => v + 1) }}><Expand size={16} />{allMode ? 'Collapse All' : 'Expand All'}</button><button title="Restore visible node positions and fit the view. Expanded branches stay open. Previous positions can be restored." onClick={resetLayout}><RotateCcw size={15} />Reset Layout</button>{layoutSnapshot && <button onClick={() => { const saved = updatePositions(layoutSnapshot, true); setLayoutSnapshot(null); setFitToken((v) => v + 1); setLayoutNotice(saved ? 'Previous node positions restored.' : 'Previous positions restored for this session.') }}>Restore previous layout</button>}<button onClick={() => setFocus(true)}><Focus size={16} />Focus Mode</button></div>}
    {!focus && page === 'World Tree' && layoutNotice && <div className="layout-notice" role="status">{layoutNotice}<button aria-label="Dismiss layout notice" onClick={() => setLayoutNotice('')}><X size={12} /></button></div>}
    {focus && <button className="exit-focus" onClick={() => setFocus(false)}><ArrowLeft size={16} />Exit focus <kbd>Esc</kbd></button>}
    {!focus && searchVisible && <div id="capability-search-results" ref={searchResults} className="search-palette" aria-label="Capability search results">
      <p role="status">{searchMatches.length ? searchMatches.length + ' matching capabilities' : 'No capabilities match this search.'}</p>
      {searchMatches.map((item, index) => <button key={item.id} onClick={() => { locateCapability(item, true); searchInput.current?.focus() }} onKeyDown={(event) => {
        if (event.key === 'Escape' || event.key === 'ArrowUp' && index === 0) {
          event.preventDefault()
          if (event.key === 'Escape') setSearchOpen(false)
          searchInput.current?.focus()
        } else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
          event.preventDefault()
          focusSearchResult(index + (event.key === 'ArrowDown' ? 1 : -1))
        }
      }}><span>{item.name}</span><small>{item.origin === 'local' ? 'Local' : 'Discover'}</small></button>)}
    </div>}
    {!focus && page === 'Settings' && <section className="display-telemetry" aria-label="Display status">
      <span className="display-telemetry-icon" aria-hidden="true"><Orbit size={30} /></span>
      <div><h2>{use2d ? 'Lightweight 2D' : reducedQuality ? 'Performance 3D' : 'Universe display'}</h2>
        <p>{use2d ? 'Capability navigation is available in 2D.' : 'Your display settings apply to the shared universe.'}</p>
        <dl><div><dt>Local data</dt><dd>{database}</dd></div><div><dt>Star field</dt><dd>{use2d ? '2D' : metrics ? metrics.stars.toLocaleString() + ' active stars' : 'Measuring'}</dd></div></dl>
      </div>
    </section>}
    {!focus && details && <aside className="spatial-pod" aria-label="Capability intelligence"><div className="pod-heading"><div><SparkleMark /><span>{page === 'Settings' ? 'Display & connections' : page === 'Learning' ? 'Learning workspace' : 'Skill Intelligence'}</span></div><button aria-label="Close details" onClick={() => setDetails(false)}><X size={17} /></button></div>
      {page === 'Settings' ? <SettingsConsole settings={settings} tab={settingsTab} onTab={setSettingsTab} onChange={updateSettings} desktop={desktop} scan={scan} database={database} metrics={metrics} backup={backup} onBackup={() => { setBusy(true); void createDesktopDatabaseBackup().then(setBackup).catch(() => reportError('Verified backup could not be created.')).finally(() => setBusy(false)) }} busy={busy} onGuide={() => setGuideStep(readWelcome()?.step ?? 0)} onDiagnostics={() => setShowDiagnostics(true)} />
        : page === 'Learning' ? <LearningJourney capability={selectedCapability} capabilities={capabilities} onChoose={(id) => setSelectedCapabilityId(id)} stage={stage} setStage={(index) => { setStage(index); setSelectedId('stage:' + index) }} record={selectedCapability ? learning[selectedCapability.entityId] : undefined} desktop={desktop} busy={busy} onUpdate={updateLearning} onProgress={publishLearningView} onLocate={() => { if (selectedCapability) locateCapability(selectedCapability) }} onRecommendChoose={logicalId=>{const next=capabilities.find(c=>(c.logicalId??('logical:github:'+c.entityId))===logicalId);if(next)setSelectedCapabilityId(next.id)}} />
        : <>{page === 'Installed' && <div className="scan-console"><div><i className={scan ? 'local-dot' : 'candidate-dot'} />{scan ? 'Last scan · ' + new Date(scan.scannedAtUnixMs).toLocaleString() : 'No local scan yet'}</div><button disabled={!desktop || scanning} onClick={() => void scanAgain()}><RefreshCw size={14} />{scanning ? 'Scanning…' : 'Scan Again'}</button>{scanError && <p role="alert" className="console-error scan-error">{scanError}</p>}</div>}
        {selectedCapability && selectedId === selectedCapability.id ? <><CapabilityDetails capability={selectedCapability} onInspect={(id)=>{const item=capabilities.find(c=>(c.logicalId??('logical:github:'+c.entityId))===id);if(item)locateCapability(item)}} onLearn={() => changePage('Learning')} onWorld={page !== 'World Tree' ? () => locateCapability(selectedCapability) : undefined} onInstalled={page !== 'Installed' && selectedCapability.origin === 'local' ? () => locateInstalled(selectedCapability) : undefined} onMission={() => setMissionOpen(true)} /><GitHubCapabilityControls key={selectedCapability.id} capability={selectedCapability} onChanged={reloadCatalog} /></>
          : page === 'Discover' ? <><div className="pod-subject"><Compass size={34} /><div><small>Codex capability discovery</small><h1>{functionCategory ? vaultType ? typeLabels[vaultType as CapabilityKind] : functionLabels[functionCategory] : 'Explore capabilities'}</h1></div></div>
            <p className="pod-description">{functionCategory ? 'Choose a capability to inspect its source, requirements, and Codex integration.' : 'Explore twenty functional categories and candidates awaiting review. Skills and Plugins share stable identities across categories.'}</p>
            {!functionCategory && <><div className="discovery-totals"><span><strong>{discoveryCategories.length}</strong>categories</span><span><strong>{candidates.filter((item) => item.kind === 'skill').length}</strong>Skills</span><span><strong>{candidates.filter((item) => item.kind === 'plugin').length}</strong>Plugins</span></div><p className="catalog-note">Counts describe distinct entries. A plugin can serve several categories. Repository stars are shared by its bundled components.</p></>}
            <GitHubDiscoveryControls onChanged={reloadCatalog} /><div className="discovery-directory" aria-label="Discovery directory">{nodes.filter((node) => node.id !== 'codex-root').map((node) => <button key={node.id} aria-label={'Browse ' + node.name} onClick={() => select(node)}><span>{node.name}<small>{node.capability ? node.capability.item?.repositoryStars !== undefined ? node.capability.item.repositoryStars.toLocaleString() + ' repo stars · setup required' : node.capability.kind : !functionCategory ? candidates.filter((item) => belongsToFunction(item, node.id.slice(9)) && item.kind === 'skill').length + ' Skills · ' + candidates.filter((item) => belongsToFunction(item, node.id.slice(9)) && item.kind === 'plugin').length + ' Plugins' : node.count + ' capabilities'}</small></span><ChevronRight size={15} /></button>)}</div>
            <p className="catalog-note">Source snapshot checked {new Date(discoveryVerifiedAt).toLocaleDateString()}. GitHub provides no universal component rating. Installation and account access are checked separately.</p></>
            : page === 'Installed' ? <><div className="pod-subject"><Database size={34} /><div><small>Local capability vault</small><h1>Local Codex</h1></div></div><p className="pod-description">{desktop ? 'Read-only discovery of your actual Skills, MCP configuration, and plugin metadata. Findings are persisted in SQLite.' : 'The browser preview cannot inspect your Codex folders. Open the desktop app to scan.'}</p><div className="vault-types" aria-label="Local capability directory">{nodes.filter((node) => node.kind !== 'core').map((node) => <button key={node.id} aria-label={'Open ' + node.name} onClick={() => select(node)}><span>{node.name}{node.capability && <small>{stateLabels[node.capability.installation]} · {node.capability.kind}</small>}</span><strong>{node.count}</strong><ChevronRight size={13} /></button>)}</div><div className="pod-section"><h2>Codex environment</h2><dl><div><dt>Codex home</dt><dd>{scan ? scan.codexHomeFound ? 'Found' : 'Not found' : 'Not scanned'}</dd></div><div><dt>CLI on PATH</dt><dd>{scan ? scan.cliFoundOnPath ? 'Found' : 'Not found' : 'Not scanned'}</dd></div><div><dt>Storage</dt><dd>{database}</dd></div></dl></div><div className="pod-section"><h2>Truthful states</h2><p>Valid local manifests are marked found. MCP entries remain configured and unverified. No scanned code or server is started.</p></div></>
              : <><div className="pod-subject"><Orbit size={34} /><div><small>{selectedNode?.kind === 'domain' ? 'Functional category' : selectedNode?.kind === 'cluster' ? 'Capability type' : 'Your capability universe'}</small><h1>{selectedNode?.name ?? 'MY CODEX'}</h1></div></div><p className="pod-description">Explore the neural branches. Each light is a capability with its own source, status, and learning path.</p><div className="source-counts"><span><i className="local-dot" />Local capabilities<strong>{local.length}</strong></span><span><i className="candidate-dot" />Discovery candidates<strong>{candidates.length}</strong></span></div><div className="pod-section"><h2><Layers3 size={17} />Progressive exploration</h2><p>Choose a functional category, then Skills, Plugins, or MCP. One type opens at a time. Its capabilities appear together in the fourth column. Drag empty space to pan through the tree.</p></div><div className="pod-section"><h2><ShieldCheck size={17} />Observed installation state</h2><p>{scan ? 'Local findings restored from the scan. MCP configuration does not verify a running connection.' : 'Scan your local Codex environment to add installed capabilities.'}</p></div><div className="pod-section"><h2><BookOpen size={17} />Learning stays separate</h2><p>Select any capability to begin. Installation and self-reported practice do not imply mastery.</p></div><button className="pod-primary" onClick={() => changePage('Installed')}><Database size={17} />{scan ? 'Open local capability vault' : 'Scan local Codex'}<ArrowUpRight size={16} /></button></>}
        </>}
    </aside>}
    {!focus && !details && <button className="open-pod" onClick={() => setDetails(true)}><Eye size={17} />Details</button>}
    {!focus && <footer className="universe-footer"><div><span><i className="local-dot" />Local</span><span><i className="candidate-dot" />Discovery</span><span><i className="configured-dot" />Configured / unverified</span></div><span>{page === 'World Tree' ? local.length + ' local · ' + candidates.length + ' candidates · ' + (allMode ? 'All branches' : 'Single branch') : page}</span></footer>}
    <div className="universe-zoom" title="Left-drag empty space to pan. Right-drag to rotate. Scroll to zoom."><button aria-label="Zoom out" onClick={() => setZoom((v) => Math.max(.35, v - .15))}><Minus size={16} /></button><span>{Math.round(zoom * 100)}%</span><button aria-label="Zoom in" onClick={() => setZoom((v) => Math.min(2, v + .15))}><Plus size={16} /></button><button aria-label="Fit View" title="Fit View: adjust camera only" onClick={() => { setZoom(1); setFitToken((v) => v + 1) }}><Maximize2 size={16} /></button></div>
    {!focus && allDiagnostics.length > 0 && <button className="diagnostics-trigger" onClick={() => setShowDiagnostics(!showDiagnostics)}>Diagnostics <span>{allDiagnostics.length}</span></button>}
    {showDiagnostics && !focus && <div className="diagnostics-center" role="dialog" aria-label="Diagnostics"><div><h2>Diagnostics</h2><button aria-label="Close diagnostics" onClick={() => setShowDiagnostics(false)}><X size={16} /></button></div>{allDiagnostics.map((message) => <p key={message}>{message}</p>)}{pendingOperations.map(op=>{const item=capabilities.find(cap=>cap.entityId===op.plan.entityId);return item&&<button className="pod-secondary" key={op.id} onClick={()=>{setPage('World Tree');setQuery('');setExpanded(new Set([item.functionCategory]));setClusters(new Set(['type:'+item.functionCategory+':'+item.kind]));setSelectedId(item.id);setSelectedCapabilityId(item.id);setDetails(true);setShowDiagnostics(false);setFitToken(value=>value+1)}}>Review interrupted {item.name} · {op.plan.scope}</button>})}</div>}
    </div>
  </main>
}
function SparkleMark() { return <span className="intelligence-mark"><GitBranch size={25} /></span> }
function CapabilityDetails({ capability: item, onLearn, onInspect, onWorld, onInstalled, onMission }: { capability: Capability; onLearn: () => void; onInspect: (id: string) => void; onWorld?: () => void; onInstalled?: () => void; onMission: () => void }) {
  const description = item.summary || 'No additional description was provided by the local metadata.'
  return <><div className="pod-subject"><span className="pod-capability-icon">{item.kind === 'mcp' ? <GitBranch size={30} /> : item.kind === 'plugin' ? <Box size={30} /> : <BookOpen size={30} />}</span><div><small>{item.origin === 'local' ? 'Local capability' : 'Discovery candidate'}</small><h1>{item.name}</h1><span className="pod-source">{item.source}</span></div></div>
    <span className={'installation-label status-' + item.installation}><i />{stateLabels[item.installation]}</span><div className="pod-section capability-description"><h2>What it does</h2><p>{description.length > 320 ? description.slice(0, description.lastIndexOf(' ', 290)) + '…' : description}</p>{description.length > 320 && <details><summary>Read full description</summary><p>{description}</p></details>}</div>
    {item.item?.compatibilityEvidence && <div className="pod-section discovery-evidence"><h2>Codex integration</h2><p>Codex {item.kind === 'plugin' ? 'plugin manifest' : 'Skill format'} checked at the pinned source revision. Runtime setup is required.</p><dl><div><dt>Repository stars</dt><dd>{item.item.repositoryStars?.toLocaleString()}</dd></div><div><dt>Repository last pushed</dt><dd>{item.item.repositoryPushedAt?.slice(0, 10)}</dd></div><div><dt>Source checked</dt><dd>{item.item.verifiedAt?.slice(0, 10)}</dd></div><div><dt>Source revision</dt><dd>{item.item.sourceRef?.slice(0, 12)}</dd></div></dl><p className="pod-muted">Repository metrics apply to the shared collection, not an individual Skill rating.</p><a href={item.item.compatibilityEvidence} target="_blank" rel="noreferrer">Inspect Codex manifest<ArrowUpRight size={13} /></a><ul>{item.item.setupRequirements?.map((requirement) => <li key={requirement}>{requirement}</li>)}</ul></div>}
    <IntelligencePanel key={item.entityId} capability={item} onInspect={onInspect}/><div className="pod-section"><h2>Capability details</h2><dl><div><dt>Type</dt><dd>{item.kind}</dd></div><div><dt>Functions</dt><dd>{(item.functionCategories ?? [item.functionCategory]).map((key) => functionLabels[key]).join(', ')}</dd></div><div><dt>Classification</dt><dd>{item.classification}</dd></div><div><dt>License</dt><dd>{item.item?.license ?? 'See local source'}</dd></div><div><dt>Learning</dt><dd>{item.learningStatus.replaceAll('_', ' ').toLowerCase()}</dd></div></dl></div><div className="pod-section"><h2>Learning path</h2><p>Explore → Configure → Practice → Verify → Master</p><p className="pod-muted">Verified progress requires saved evidence.</p></div><button className="pod-primary" onClick={onLearn}><BookOpen size={17} />Explore learning path<ArrowUpRight size={16} /></button><div className="capability-destinations">
    {onWorld && <button onClick={onWorld}><GitBranch size={15} />Locate in World Tree</button>}
    {onInstalled && <button onClick={onInstalled}><Box size={15} />View in Installed</button>}
    <button onClick={onMission}><ArrowUpRight size={15} />Use in Mission Control</button>
    {item.sourceUrl && <a href={item.sourceUrl} target="_blank" rel="noreferrer">Open GitHub<ArrowUpRight size={14} /></a>}
  </div></>
}

function LearningJourney({ capability, capabilities, onChoose, stage, setStage, record, desktop, busy, onUpdate,onLocate,onRecommendChoose,onProgress }: {
  capability?: Capability; capabilities: Capability[]; onChoose: (id: string) => void; stage: number; setStage: (index: number) => void
  record?: StoredLearningRecord; desktop: boolean; busy: boolean; onUpdate: (status: 'EXPLORING' | 'SELF_REPORTED') => Promise<void>;onLocate:()=>void;onRecommendChoose:(id:string)=>void;onProgress:(entityId:string,labels:string[])=>void
}) {
  const [engineReady,setEngineReady]=useState(false)
  const [note, setNote] = useState('')
  const [savedNote, setSavedNote] = useState('')
  const [saving, setSaving] = useState(false)
  const [failure, setFailure] = useState('')
  const descriptions = [
    'Understand what the capability does, review its source, and choose one small use case.',
    'Review the official setup instructions and permissions. Check the observed installation state before proceeding.',
    'Complete a small task with the capability in your own workflow. Save what you tried and the result.',
    'Review saved practice evidence. A note or self-report is not an independent verification.',
    'Apply the capability across several tasks and review the results. Mastery is not yet verified for this capability.',
  ]
  const saveNote = async () => {
    if (!capability || !desktop || !note.trim()) return
    setSaving(true); setFailure('')
    try {
      await saveDesktopPracticeEvidence(capability.entityId, note.trim())
      setSavedNote(note.trim()); setNote('')
    } catch { setFailure('Practice evidence could not be saved. Your text is still available.') }
    finally { setSaving(false) }
  }
  let storedEvidence = ''
  try { const data = JSON.parse(record?.evidenceJson ?? '[]'); storedEvidence = Array.isArray(data) ? data.filter((item) => item.type === 'user-practice-note').map((item) => item.content).join('\n') : '' } catch { /* Optional stored evidence remains unverified. */ }
  return <><div className="pod-subject"><GraduationCap size={34} /><div><small>Learning journey</small><h1>Your learning path</h1></div></div><label className="console-field"><span>Capability</span><select aria-label="Learning capability" value={capability?.id ?? ''} onChange={(event) => { onChoose(event.target.value); setStage(0); setSavedNote(''); setNote('') }}><option value="">Choose a capability</option>{capabilities.map((item) => <option key={item.id} value={item.id}>{item.origin === 'local' ? 'Local · ' : 'Discovery · '}{item.name}</option>)}</select></label>
    {capability&&<LearningWorkbench key={capability.entityId} capability={capability} stage={stage} onStage={setStage} onLocate={onLocate} onChoose={onRecommendChoose} onReady={setEngineReady} onProgress={onProgress}/>}
    <details className="legacy-learning" open={!engineReady}><summary>Earlier exploration notes</summary><div className="journey-tabs">{stageNames.map((name, index) => <button key={name} aria-pressed={stage === index} onClick={() => setStage(index)}><span>{index + 1}</span>{name}</button>)}</div>
    <div className="pod-section"><h2>{stageNames[stage]}</h2><p>{descriptions[stage]}</p></div>
    <div className="learning-status">{record ? record.status.replaceAll('_', ' ').toLowerCase() : 'No learning records yet'}<small>{record ? 'Saved locally. Progress is separate from installation.' : 'Choose a capability and start exploring.'}</small></div>
    {capability && <><div className="pod-section"><h2>{capability.name}</h2><p>{capability.summary}</p><p className="pod-muted">{stateLabels[capability.installation]}</p></div>
      {stage === 0 && <button className="pod-primary" disabled={!desktop || busy} onClick={() => void onUpdate('EXPLORING')}>Start learning<ArrowUpRight size={16} /></button>}
      {stage === 1 && capability.sourceUrl && <a className="pod-secondary" href={capability.sourceUrl} target="_blank" rel="noreferrer">Review setup instructions<ArrowUpRight size={15} /></a>}
      {stage === 2 && <><label className="console-field"><span>Practice evidence</span><textarea aria-label="Practice evidence" placeholder="What did you try? What result did you observe?" value={note} maxLength={4000} onChange={(event) => setNote(event.target.value)} /></label><button className="pod-primary" disabled={!desktop || saving || !note.trim()} onClick={() => void saveNote()}>{saving ? 'Saving…' : 'Save practice note'}</button><button className="pod-secondary" disabled={!desktop || busy} onClick={() => void onUpdate('SELF_REPORTED')}>Mark practice as self reported</button></>}
      {(stage === 2 || stage === 3) && (savedNote || storedEvidence) && <div className="evidence-note"><strong>Saved evidence · not verified</strong><p>{savedNote || storedEvidence}</p></div>}
      {(stage === 3 || stage === 4) && <p className="pod-description">Independent verification and mastery evaluation are not available yet. This stage remains incomplete.</p>}
      {!desktop && <p className="pod-muted">Learning changes are saved in the desktop SQLite database. Browser preview is read-only.</p>}
    </>}
    {failure && <p role="alert" className="console-error">{failure}</p>}
  </details></>
}
const ignoreModelSelection = () => undefined

function SettingsConsole({ settings, tab, onTab, onChange, desktop, scan, database, metrics, backup, onBackup, busy, onDiagnostics, onGuide }: {
  settings: UniverseSettings; tab: string; onTab: (tab: string) => void
  onChange: <K extends keyof UniverseSettings>(key: K, value: UniverseSettings[K]) => void
  desktop: boolean; scan: CodexScanReport | null; database: string; metrics: RenderMetrics | null
  backup: string; onBackup: () => void; busy: boolean; onDiagnostics: () => void; onGuide: () => void
}) {
  return <><div className="pod-subject"><Settings2 size={34} /><div><small>Spatial control console</small><h1>Universe settings</h1></div></div><div className="console-tabs">{['Display', 'Local Codex', 'AI Providers', 'Privacy'].map((name) => <button key={name} aria-pressed={name === tab} onClick={() => onTab(name)}>{name}</button>)}</div>
    {tab === 'AI Providers' && (desktop ? <ModelRegistryPanel settings disabled={busy} complex={false} onSelection={ignoreModelSelection}/> : <p>Open the Windows desktop app to manage secure model credentials and budgets.</p>)}
    {tab === 'Display' && <><label className="console-field"><span>Rendering mode</span><select aria-label="Rendering mode" value={settings.rendering} onChange={event => onChange('rendering', event.target.value as UniverseSettings['rendering'])}><option value="auto">Automatic</option><option value="3d">Keep 3D</option><option value="2d">Lightweight 2D</option></select></label><label className="console-toggle"><span>Star field</span><input type="checkbox" aria-label="Star field" checked={settings.stars} onChange={event => onChange('stars',event.target.checked)}/></label><label className="console-toggle"><span>Particles</span><input type="checkbox" aria-label="Particles" checked={settings.particles} onChange={event => onChange('particles',event.target.checked)}/></label><label className="console-toggle"><span>Nebula field<small>Three-dimensional gas volumes in the shared universe.</small></span><input type="checkbox" aria-label="Nebula field" checked={settings.nebula} onChange={(event) => onChange('nebula', event.target.checked)} /></label><label className="console-field"><span>Bloom intensity<small>Only HDR highlights bloom; Performance mode disables this pass.</small></span><input type="range" aria-label="Bloom intensity" min="0" max=".8" step=".05" value={settings.bloom} onChange={(event) => onChange('bloom', Number(event.target.value))} /><output>{Math.round(settings.bloom * 100)}%</output></label><label className="console-field"><span>3D Quality<small>Controls render resolution, gas sampling, and the post-processing budget.</small></span><select aria-label="3D Quality" value={settings.quality} onChange={(event) => onChange('quality', event.target.value as UniverseSettings['quality'])}><option value="low">Performance</option><option value="balanced">Balanced</option><option value="high">High · bloom</option></select></label><label className="console-field"><span>Particle Density<small>Adjusts Drei Stars and Sparkles in the active scene.</small></span><input type="range" aria-label="Particle Density" min=".15" max="1.5" step=".05" value={settings.density} onChange={(event) => onChange('density', Number(event.target.value))} /><output>{Math.round(settings.density * 100)}%</output></label><label className="console-field"><span>Motion Intensity<small>Applies to particles, orbit drift, and camera transitions.</small></span><input type="range" aria-label="Motion Intensity" min="0" max="1" step=".05" value={settings.motion} onChange={(event) => onChange('motion', Number(event.target.value))} /><output>{Math.round(settings.motion * 100)}%</output></label><label className="console-toggle"><span>Reduced Motion<small>Stops ambient motion and uses on-demand rendering.</small></span><input type="checkbox" aria-label="Reduced Motion" checked={settings.reducedMotion} onChange={(event) => onChange('reducedMotion', event.target.checked)} /></label><div className="pod-section"><h2>Live renderer</h2><p>{metrics ? metrics.renderer + ' · ' + (metrics.fps ? metrics.fps + ' fps · P95 ' + metrics.p95 + ' ms' : 'on-demand rendering') + ' · ' + metrics.calls + ' draw calls' : 'Measuring the active renderer…'}</p><p className="pod-muted">{metrics ? metrics.stars + ' distant stars · ' + metrics.sparkles + ' spatial particles' : 'Drei Stars and Sparkles are active in the shared scene.'}</p></div></>}
    {tab === 'Local Codex' && <><div className="pod-section"><h2>Local environment</h2><dl><div><dt>Mode</dt><dd>{desktop ? 'Native desktop' : 'Browser preview'}</dd></div><div><dt>Codex home</dt><dd>{scan ? scan.codexHomeFound ? 'Detected' : 'Not found' : 'Not scanned'}</dd></div><div><dt>Codex CLI</dt><dd>{scan ? scan.cliFoundOnPath ? 'Detected' : 'Not found' : 'Not scanned'}</dd></div></dl></div><label className="console-toggle"><span>Restore last scan<small>Load saved findings on the next application launch.</small></span><input type="checkbox" aria-label="Restore last scan" checked={settings.restoreScan} onChange={(event) => onChange('restoreScan', event.target.checked)} /></label><label className="console-toggle"><span>Scan on startup<small>Run the read-only Codex scan on the next launch.</small></span><input type="checkbox" aria-label="Scan on startup" checked={settings.autoScan} onChange={(event) => onChange('autoScan', event.target.checked)} /></label><div className="pod-section"><h2>Data storage</h2><p>{database}. Capability metadata, scans, and learning records use SQLite. Display preferences and spatial layout use this WebView's local storage.</p></div><button className="pod-secondary" disabled={!desktop || busy} onClick={onBackup}>{busy ? 'Creating backup…' : 'Create verified SQLite backup'}</button>{backup && <p className="console-success" role="status">Backup verified: {backup}</p>}</>}
    {tab === 'Privacy' && <><div className="pod-section"><h2>Local-first operation</h2><p>Scanning reads metadata. Skills are not executed, MCP servers are not launched, and Codex configuration is not edited.</p></div><div className="pod-section"><h2>AI provider</h2><p>Live model requests require explicit consent. Provider keys are saved in the operating system credential store.</p></div><div className="pod-section"><h2>Diagnostics</h2><p>Inspect application and database errors in the diagnostics center. Sensitive configuration values are not displayed.</p></div><button className="pod-secondary" onClick={onDiagnostics}>Open diagnostics</button><button className="pod-secondary" onClick={onGuide}>Open getting started</button></>}
  </>
}
