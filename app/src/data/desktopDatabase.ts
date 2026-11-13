import type { LocalIdentityScan } from './capabilityIdentity'
import { loadInstallationState, emptyInstallationState, type InstallationState } from './installationManager'
import { invoke } from '@tauri-apps/api/core'
import { Boxes } from 'lucide-react'
import { branches, catalog, retiredCatalogIds, type CatalogItem, type Category } from '../domain/catalog'
import { emptyDiscoveryState, loadGitHubState, type DiscoveryState } from './githubDiscovery'

export interface StoredGraphPosition {
  id: string
  x: number | null
  y: number | null
  pinned: boolean
}

interface StoredCapability {
  id: string
  kind: string
  name: string
  summary: string
  sourceType: string | null
  sourceOwner: string | null
  sourceRepo: string | null
  sourceRef: string | null
  sourceUrl: string | null
  licenseSpdx: string | null
  metadataJson: string
  primaryCategoryId: string | null
  primaryCategoryName: string | null
}

interface StoredDomain {
  id: string
  name: string
}

export interface StoredLearningRecord {
  id: string
  entityId: string
  status: 'NOT_LEARNED' | 'EXPLORING' | 'PRACTICING' | 'VERIFIED' | 'SELF_REPORTED'
  evidenceJson: string
  updatedAt: string
}

export interface CodexScanReport {
  identity?: LocalIdentityScan
  codexHomeFound: boolean
  cliFoundOnPath: boolean
  scannedAtUnixMs: number
  skills: Array<{ id: string; name: string; description: string; scope: string; enabled: boolean | null; valid: boolean }>
  mcpServers: Array<{ id: string; name: string; scope: string; transport: string; enabled: boolean; verification: string }>
  plugins: Array<{ id: string; name: string; version: string | null; enabled: boolean | null; valid: boolean }>
  warnings: string[]
}

export async function scanDesktopCodex(): Promise<CodexScanReport> {
  return invoke<CodexScanReport>('scan_codex')
}

export async function loadDesktopCodexScan(): Promise<CodexScanReport | null> {
  return invoke<CodexScanReport | null>('get_codex_scan')
}

export async function loadDesktopLearningProgress(id: string): Promise<StoredLearningRecord | null> {
  return invoke<StoredLearningRecord | null>('get_learning_progress', { id })
}

export async function saveDesktopLearningProgress(id: string, status: 'EXPLORING' | 'SELF_REPORTED'): Promise<StoredLearningRecord> {
  return invoke<StoredLearningRecord>('update_learning_progress', {
    input: {
      id: `learning-${id}`,
      entityId: id,
      status,
      evidence: status === 'SELF_REPORTED' ? [{ type: 'user-self-report', recordedAt: new Date().toISOString() }] : [],
    },
  })
}

export async function saveDesktopPracticeEvidence(id: string, content: string): Promise<StoredLearningRecord> {
  return invoke<StoredLearningRecord>('update_learning_progress', {
    input: { id: 'learning-' + id, entityId: id, status: 'PRACTICING', evidence: [{ type: 'user-practice-note', content: content.slice(0, 4000), recordedAt: new Date().toISOString(), verified: false }] },
  })
}

export async function checkDesktopDatabase(): Promise<void> {
  await invoke<void>('check_database_integrity')
}

export async function createDesktopDatabaseBackup(): Promise<string> {
  return invoke<string>('create_database_backup')
}

const catalogById = new Map(catalog.map((item) => [item.id, item]))

function firstByKey<T>(entries: T[], key: (item: T) => string): Map<string, T> {
  const result = new Map<string, T>()
  for (const entry of entries) {
    const id = key(entry)
    if (!result.has(id)) result.set(id, entry)
  }
  return result
}

export async function loadDesktopCatalog(): Promise<{ items: CatalogItem[]; domains: Category[]; githubState: DiscoveryState; installationState: InstallationState; warnings: string[] }> {
  const [storedItems, storedDomains, integration] = await Promise.all([
    invoke<StoredCapability[]>('list_capabilities'),
    invoke<StoredDomain[]>('list_domains'),
    Promise.allSettled([loadGitHubState(), loadInstallationState()]),
  ])
  const [github, installation] = integration
  const warnings: string[] = []
  if (github.status === 'rejected') warnings.push('GitHub cache could not be loaded. Repository metadata may be unavailable.')
  if (installation.status === 'rejected') warnings.push('Installation history could not be read. Managed installation state is unavailable.')
  const githubState = github.status === 'fulfilled' ? github.value : emptyDiscoveryState
  const installationState = installation.status === 'fulfilled' ? installation.value : emptyInstallationState
  const knownDomains = new Set<Category>(branches)
  const domains = storedDomains.flatMap(({ name }) => knownDomains.has(name as Category) ? [name as Category] : [])
  const items = storedItems.flatMap((record) => {
    // Scanner entities are indexed from the saved scan, preserving their local provenance.
    if (record.sourceType === 'local' || retiredCatalogIds.has(record.id)) return []
    const fallback = catalogById.get(record.id)
    const rawCategory = record.primaryCategoryName
    const category: Category = rawCategory && knownDomains.has(rawCategory as Category) ? rawCategory as Category : 'Unclassified'
    const owner = record.sourceOwner ?? fallback?.owner ?? ''
    const repo = record.sourceRepo ?? fallback?.repo ?? ''
    let capabilities = fallback?.capabilities ?? []
    let discovery: Partial<CatalogItem> = {}
    try {
      const metadata = JSON.parse(record.metadataJson) as { capabilities?: unknown; discovery?: Partial<CatalogItem> }
      if (Array.isArray(metadata.capabilities)) capabilities = metadata.capabilities.filter((item): item is string => typeof item === 'string').slice(0, 8)
      if (metadata.discovery?.id === record.id) discovery = metadata.discovery
    } catch { /* Metadata is optional display detail; the stored record remains usable. */ }
    return [{
      ...fallback, ...discovery,
      id: record.id,
      name: record.name,
      owner,
      repo,
      githubUrl: record.sourceUrl ?? fallback?.githubUrl ?? (owner && repo ? `https://github.com/${owner}/${repo}` : ''),
      summary: record.summary,
      category,
      capabilities,
      license: record.licenseSpdx ?? fallback?.license ?? 'License not verified',
      mark: fallback?.mark ?? record.kind,
      Icon: fallback?.Icon ?? Boxes,
      treePath: fallback?.treePath,
    } satisfies CatalogItem]
  })
  const repositories = new Map<string, DiscoveryState['repositories'][number]>()
  for (const repository of githubState.repositories) {
    for (const name of [repository.fullName.toLowerCase(), ...(repository.aliases ?? [])]) {
      if (!repositories.has(name)) repositories.set(name, repository)
    }
  }
  const assessments = firstByKey(githubState.assessments, (entry) => entry.entityId)
  const classifications = firstByKey(githubState.classifications, (entry) => entry.entityId)
  const installations = firstByKey(installationState.records.filter((record) => record.state !== 'UNINSTALLED'), (entry) => entry.entityId)
  const enriched = items.map((item) => {
    const name = (item.owner + '/' + item.repo).toLowerCase()
    const repository = repositories.get(name)
    const assessment = assessments.get(item.id)?.data
    const classification = classifications.get(item.id)
    const timestamp = item.verifiedAt as string | number | undefined
    return { ...item, verifiedAt: typeof timestamp === 'number' ? new Date(timestamp).toISOString() : timestamp,
      ...(repository ? { repositoryStars: repository.metadata.stars, repositoryPushedAt: repository.metadata.pushedAt, repositoryArchived: repository.metadata.archived } : {}),
      ...(classification ? { functionCategories: classification.primary ? [classification.primary, ...classification.related] : ['unclassified'] } : {}),
      managedInstallation: installations.get(item.id),
      github: { repository, assessment, classification },
    }
  })
  return { items: enriched, domains, githubState, installationState, warnings }
}

export async function loadDesktopGraphLayout(profile = 'world-tree'): Promise<Record<string, { x: number; y: number }>> {
  const rows = await invoke<StoredGraphPosition[]>('load_graph_layout', { profile })
  return Object.fromEntries(rows.flatMap((row) => row.x === null || row.y === null ? [] : [[row.id, { x: row.x, y: row.y }]]))
}

export async function saveDesktopGraphLayout(positions: Record<string, { x: number; y: number }>, profile = 'world-tree'): Promise<void> {
  await invoke<void>('save_graph_layout', {
    input: {
      profile,
      positions: Object.entries(positions).map(([id, point]) => ({ id, x: point.x, y: point.y, pinned: false })),
    },
  })
}
