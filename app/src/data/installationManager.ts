import { invoke } from '@tauri-apps/api/core'
export interface InstallProfile { id: string; isolated: boolean; userHome: string; codexHome: string; projectRoot: string; workRoot: string }
export interface InstallSource { repository: string; commit: string; name: string; kind: string; fingerprint: string; files: Array<{path: string; relative: string; blob: string}> }
export interface ConfigPatch { format: string; section: string; key: string; expectedHash: string; value: Record<string, unknown> | null }
export interface InstallChange { target: string; stagingPath: string; holdingPath: string; role: string; before: {kind: string; fingerprint: string; files: Array<{path: string; sha256: string; size: number}>}; patch?: ConfigPatch }
export interface ManagedInstallation { id: string; entityId: string; profileId: string; kind: string; scope: string; target: string; source?: InstallSource; state: string; observedState?: string; verification: string[]; localEntityId?: string; currentOperationId: string; patches: Array<{target:string;patch:ConfigPatch}> }
export interface McpDraft { name: string; transport: string; command?: string; args: string[]; url?: string; environmentNames: string[]; bearerTokenEnvironment?: string; authRequired: boolean }
export interface InstallRequest { entityId: string; action: string; scope: string; alias?: string; rollbackOperation?: string; mcp?: McpDraft }
export interface InstallPlan { operationId: string; profile: InstallProfile; entityId: string; action: string; scope: string; kind: string; source?: InstallSource; sourceChanges?: Array<{path:string;state:string;script:boolean;beforeBlob?:string;afterBlob?:string}>; changes: InstallChange[]; blockers: string[]; warnings: string[]; requirements: string[]; createdAt: number; expiresAt: number; canExecute: boolean; dryRun: false }
export interface InstallOperation { id: string; state: string; plan: InstallPlan; digest: string; error?: {kind: string; message: string; retryAt?: number}; updatedAt: number; recovery: Record<string, unknown>; progress?: {completedFiles?:number;totalFiles?:number;bytes?:number;path?:string} }
export interface InstallationState { profile?: InstallProfile; records: ManagedInstallation[]; history: Array<{operation:InstallOperation;events:Array<{state:string;details:Record<string,unknown>;at:number}>}>; runtimeExecutionSupported: false }
export const emptyInstallationState: InstallationState = { records: [], history: [], runtimeExecutionSupported: false }
export const loadInstallationState = () => invoke<InstallationState>('installation_state')
export const prepareInstallation = (request: InstallRequest) => invoke<InstallOperation>('installation_prepare', { request })
export const executeInstallation = (operationId: string, planDigest: string) => invoke<InstallOperation>('installation_execute', { operationId, planDigest, confirmed: true })
export const cancelInstallation = (operationId: string) => invoke<void>('installation_cancel', { operationId })
export const recoverInstallation = (operationId: string, planDigest: string) => invoke<InstallOperation>('installation_recover', { operationId, planDigest, confirmed: true })
export const verifyInstallation = (entityId: string) => invoke<{record:ManagedInstallation;findings:Array<{target:string;matchesOwnedBytes?:boolean;matchesOwnedEntry?:boolean}>;runtimeVerified:false}>('installation_verify', { entityId })

export const loadInstallationOperation = (operationId: string) => invoke<InstallOperation>('installation_operation', { operationId })
