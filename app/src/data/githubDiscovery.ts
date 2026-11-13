import { invoke } from '@tauri-apps/api/core'

export interface GitHubError { kind: string; message: string; retryAt?: number; status?: number }
export interface GitHubRepository { id: number; fullName: string; description: string; stars: number; forks: number; archived: boolean; disabled: boolean; license?: string; pushedAt?: string; defaultBranch: string; htmlUrl: string }
export interface Classification { entityId: string; primary: string | null; related: string[]; state: string; confidence: number; evidence: string[]; manual: boolean }
export interface Intelligence { compatibility: string; reviewState: string; compatibilityEvidence: string[]; requirements: string[]; risks: string[]; checkedAt: number; latestCommit: string; latestBlob: string; updateState?: string; quality?: { score: number; maximum: number; method: string; lastCommitAt?: string }; dependencies: string[] }
export interface DiscoveryState {
  repositories: Array<{ id: number; fullName: string; metadata: GitHubRepository; fetchedAt: number; validatedAt: number; availability: string; aliases?: string[] }>
  assessments: Array<{ entityId: string; data: Intelligence; checkedAt: number }>
  classifications: Classification[]
  rateLimits: Array<{ scope: string; resource: string; remaining?: number; limitValue?: number; resetAt: number; blockedUntil: number }>
  history: Array<{ id: string; status: string; startedAt: number; finishedAt?: number; retryAt?: number; operation: GitHubOperation; summary: { requests?: number; completedComponents?: number; manifestCount?: number; currentManifest?: string; error?: GitHubError } }>
  localLinks: Array<{ localEntityId: string; remoteEntityId: string; checkedAt: number }>
  tokenConfigured: boolean
}
export interface GitHubOperation { action: 'search' | 'inspect' | 'refresh'; query?: string; repository?: string; page?: number; force?: boolean; entityId?: string }
export interface SearchResult { repositories?: GitHubRepository[]; totalCount?: number; page?: number; hasNext?: boolean; source?: string; fetchedAt?: number; validatedAt?: number; warning?: GitHubError; components?: Array<{ entityId: string; name: string; compatibility: string }>; remainingManifestCount?: number; nextManifestPage?: number; repository?: GitHubRepository }
export interface InstallationPlan { dryRun: true; canExecute: false; sourceRepository: string; sourcePath?: string; commit?: string; kind: string; targetDirectory: string; localState: string; configurationRequired: boolean; requirements?: string[]; risks?: string[]; blockers: string[]; warnings: string[]; writePreview: Array<{ sourcePath: string; targetPath: string; conflict: boolean }>; dependencies: Array<{ command: string; foundOnPath: boolean }> }
export const emptyDiscoveryState: DiscoveryState = { repositories: [], assessments: [], classifications: [], rateLimits: [], history: [], localLinks: [], tokenConfigured: false }
export const loadGitHubState = () => invoke<DiscoveryState>('github_discovery_state')
export const runGitHubOperation = (requestId: string, operation: GitHubOperation) => invoke<SearchResult>('github_run', { requestId, operation })
export const cancelGitHubOperation = (requestId: string) => invoke<void>('github_cancel', { requestId })
export const resumeGitHubOperation = (requestId: string, runId: string) => invoke<SearchResult>('github_resume', { requestId, runId })
export const setGitHubToken = (token: string | null) => invoke<void>('github_set_token', { token })
export const classifyGitHubCapability = (entityId: string, primary: string | null, related: string[]) => invoke<void>('github_classify', { entityId, primary, related })
export const previewGitHubInstallation = (entityId: string) => invoke<InstallationPlan>('github_installation_plan', { entityId })
export const gitHubError = (error: unknown): GitHubError => typeof error === 'object' && error !== null && 'message' in error ? error as GitHubError : { kind: 'UNAVAILABLE', message: 'GitHub operation failed. Existing records were retained.' }
export const displayTime = (value?: number | string) => value ? new Date(value).toLocaleString() : 'Not checked'
