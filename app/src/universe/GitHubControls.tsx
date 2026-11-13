import { InstallerControls } from './InstallerControls'
import { useEffect, useRef, useState } from 'react'
import { isTauri } from '@tauri-apps/api/core'
import { cancelGitHubOperation, classifyGitHubCapability, displayTime, gitHubError, loadGitHubState, previewGitHubInstallation, resumeGitHubOperation, runGitHubOperation, setGitHubToken, type DiscoveryState, type GitHubOperation, type InstallationPlan, type SearchResult } from '../data/githubDiscovery'
import { discoveryCategories } from '../domain/catalog'
import type { Capability } from './model'

export function GitHubDiscoveryControls({ onChanged }: { onChanged: () => Promise<void> }) {
  const desktop = isTauri()
  const [state, setState] = useState<DiscoveryState | null>(null)
  const [query, setQuery] = useState('codex skills stars:>100 archived:false')
  const [searchedQuery, setSearchedQuery] = useState('')
  const [repository, setRepository] = useState('')
  const [result, setResult] = useState<SearchResult | null>(null)
  const [message, setMessage] = useState('Startup uses saved data. GitHub requests begin only after an action.')
  const [busy, setBusy] = useState(false)
  const [token, setToken] = useState('')
  const [clock, setClock] = useState(() => Date.now())
  useEffect(() => { const timer = window.setInterval(() => setClock(Date.now()), 1000); return () => window.clearInterval(timer) }, [])
  const request = useRef<string | null>(null)
  useEffect(() => {
    if (desktop) void loadGitHubState().then(setState).catch(() => setMessage('The native GitHub module is unavailable. Existing catalog data remains accessible.'))
    return () => { if (request.current) void cancelGitHubOperation(request.current).catch(() => {}) }
  }, [desktop])
  const perform = async (operation: GitHubOperation, resumeId?: string) => {
    const id = crypto.randomUUID(); request.current = id; setBusy(true)
    setMessage('Reading public GitHub data…')
    try {
      const value = resumeId ? await resumeGitHubOperation(id, resumeId) : await runGitHubOperation(id, operation)
      setResult(value)
      setMessage(value.warning ? value.warning.message + (value.warning.retryAt ? ' Retry after ' + displayTime(value.warning.retryAt) : '') : value.components ? value.components.length + ' manifests inspected and saved. ' + (value.remainingManifestCount ?? 0) + ' further manifests are available in the next bounded inspection.' : 'GitHub operation completed.')
      if (operation.action === 'search') setSearchedQuery(operation.query ?? '')
    } catch (failure) { const error = gitHubError(failure); setMessage(error.message + (error.retryAt ? ' Retry after ' + displayTime(error.retryAt) : '')) }
    finally { request.current = null; setBusy(false); await onChanged().catch(() => setMessage("Saved GitHub data could not be reloaded. Existing catalog remains visible.")); await loadGitHubState().then(setState).catch(() => {}) }
  }
  useEffect(() => {
    if (!desktop || !busy) return
    // Progress polling reads SQLite only; it never refreshes GitHub.
    const timer = window.setInterval(() => { void loadGitHubState().then(setState).catch(() => {}) }, 1500)
    return () => window.clearInterval(timer)
  }, [desktop, busy])
  const saveToken = async (remove = false) => {
    setBusy(true)
    try { await setGitHubToken(remove ? null : token); setState(await loadGitHubState()); setMessage(remove ? 'GitHub credential removed from Windows Credential Manager.' : 'Credential saved in Windows Credential Manager. Public read-only discovery remains the scope.') }
    catch (failure) { setMessage(gitHubError(failure).message) }
    finally { setToken(''); setBusy(false) }
  }
  const latest = state?.repositories.reduce((time, item) => Math.max(time, item.validatedAt), 0)
  return <section className="pod-section" aria-label="GitHub live discovery">
    <h2>GitHub Live Intelligence</h2>
    <p role="status">{message}</p>
    {busy && state?.history[0]?.status === 'RUNNING' && <p>{state.history[0].summary.completedComponents ?? 0} / {state.history[0].summary.manifestCount ?? '?'} manifests saved · {state.history[0].summary.requests ?? 0} HTTP requests. {state.history[0].summary.currentManifest}</p>}
    <p>Last repository validation: {displayTime(latest)}. {state?.repositories.length ?? 0} repositories saved.</p>
    {state?.rateLimits.map((rate) => <p key={rate.scope + rate.resource}>{rate.resource}: {rate.remaining ?? '?'} / {rate.limitValue ?? '?'} remaining{rate.blockedUntil > clock ? ' · paused until ' + displayTime(rate.blockedUntil) : ''}</p>)}
    <button className="pod-secondary" disabled={!desktop || busy} onClick={() => void perform({ action: 'refresh', force: true })}>Refresh repository metadata</button>
    {busy && <button className="pod-secondary" onClick={() => { if (request.current) void cancelGitHubOperation(request.current) }}>Cancel GitHub operation</button>}
    <details><summary>Search, inspect, and sync history</summary>
      <form onSubmit={(event) => { event.preventDefault(); void perform({ action: 'search', query, page: 1 }) }}>
        <label className="console-field"><span>Public repository search</span><input aria-label="Search GitHub repositories" value={query} maxLength={180} onChange={(event) => setQuery(event.target.value)} /></label>
        <button className="pod-secondary" disabled={!desktop || busy || !query.trim()}>Search GitHub</button>
      </form>
      <form onSubmit={(event) => { event.preventDefault(); void perform({ action: 'inspect', repository, force: true }) }}>
        <label className="console-field"><span>Repository to inspect</span><input aria-label="Repository to inspect" placeholder="owner/repository" value={repository} onChange={(event) => setRepository(event.target.value)} /></label>
        <button className="pod-secondary" disabled={!desktop || busy || !repository.trim()}>Inspect and save candidates</button>
      </form>
      {result?.repositories && <div aria-label="GitHub repository results"><p>{result.totalCount} repository leads. A name match does not establish compatibility. Content fetched {displayTime(result.fetchedAt)}; validated {displayTime(result.validatedAt)} ({result.source}).</p>
        {result.repositories.map((repo) => <div className="pod-section" key={repo.id}><a href={repo.htmlUrl} target="_blank" rel="noreferrer">{repo.fullName}</a><p>{repo.stars.toLocaleString()} repository stars{repo.archived ? ' · Archived' : ''}</p><p>{repo.description}</p><button className="pod-secondary" disabled={busy} onClick={() => void perform({ action: 'inspect', repository: repo.fullName })}>Inspect {repo.fullName}</button></div>)}
        {(result.page ?? 1) > 1 && <button disabled={busy} className="pod-secondary" onClick={() => void perform({ action: 'search', query: searchedQuery, page: result.page! - 1 })}>Previous GitHub result page</button>}
        {result.hasNext && <button disabled={busy} className="pod-secondary" onClick={() => void perform({ action: 'search', query: searchedQuery, page: (result.page ?? 1) + 1 })}>Next GitHub result page</button>}
      </div>}
      {result?.nextManifestPage && result.repository && <button className="pod-secondary" disabled={busy} onClick={() => void perform({ action: 'inspect', repository: result.repository!.fullName, page: result.nextManifestPage })}>Inspect next manifest batch</button>}
      {state?.history.slice(0, 5).map((run) => <div key={run.id}><p>{run.operation.action}: {run.status} · {run.summary.requests ?? 0} HTTP requests · {displayTime(run.startedAt)}</p>{run.retryAt && <p>Earliest retry: {displayTime(run.retryAt)}</p>}{['DEFERRED', 'FAILED', 'PARTIAL', 'CANCELLED', 'INTERRUPTED'].includes(run.status) && <button className="pod-secondary" disabled={busy || Boolean(run.retryAt && run.retryAt > clock)} onClick={() => void perform(run.operation, run.id)}>Retry saved operation</button>}</div>)}
    </details>
    <details><summary>Optional GitHub credential</summary><p>Anonymous public access works without a token. Use only read-only public access; no repository write permission is required. Credentials are stored in Windows Credential Manager.</p>
      <label className="console-field"><span>GitHub token</span><input type="password" aria-label="GitHub token" autoComplete="off" value={token} onChange={(event) => setToken(event.target.value)} /></label>
      <button className="pod-secondary" disabled={!desktop || busy || !token.trim()} onClick={() => void saveToken()}>Save GitHub credential</button>
      {state?.tokenConfigured && <button className="pod-secondary" disabled={busy} onClick={() => void saveToken(true)}>Remove GitHub credential</button>}
    </details>
  </section>
}

export function GitHubCapabilityControls({ capability, onChanged }: { capability: Capability; onChanged: () => Promise<void> }) {
  const desktop = isTauri()
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState('')
  const [plan, setPlan] = useState<InstallationPlan | null>(null)
  const [primary, setPrimary] = useState(capability.functionCategory === 'unclassified' ? '' : capability.functionCategory)
  const [related, setRelated] = useState((capability.functionCategories ?? []).slice(1))
  const request = useRef<string | null>(null)
  useEffect(() => () => { if (request.current) void cancelGitHubOperation(request.current).catch(() => {}) }, [])
  const inspect = async () => {
    if (!capability.item) return
    const id = crypto.randomUUID(); request.current = id; setBusy(true)
    try { const result = await runGitHubOperation(id, { action: 'inspect', repository: capability.item.owner + '/' + capability.item.repo, entityId: capability.entityId, force: true }); setMessage(result.components ? 'Source inspection saved.' : 'Inspection completed.'); await onChanged() }
    catch (error) { setMessage(gitHubError(error).message) }
    finally { request.current = null; setBusy(false) }
  }
  const preview = async () => { setBusy(true); try { setPlan(await previewGitHubInstallation(capability.entityId)) } catch (error) { setMessage(gitHubError(error).message) } finally { setBusy(false) } }
  const saveCategory = async () => { setBusy(true); try { await classifyGitHubCapability(capability.entityId, primary || null, related.filter((key) => key !== primary)); await onChanged(); setMessage('Classification saved. Installation evidence was retained.') } catch (error) { setMessage(gitHubError(error).message) } finally { setBusy(false) } }
  const info = capability.item?.github?.assessment
  return <><InstallerControls capability={capability} onChanged={onChanged} /><section className="pod-section" aria-label="Capability intelligence">
    <h2>Live source assessment</h2><p>{info?.compatibility ?? 'UNVERIFIED'} · {info?.reviewState ?? 'Source not inspected live'}</p>
    {info && <><p>Checked {displayTime(info.checkedAt)}. {info.updateState?.replaceAll('_', ' ')}</p>{info.quality && <p>Suitability {info.quality.score}/{info.quality.maximum}. {info.quality.method}</p>}<ul>{info.risks?.map((risk) => <li key={risk}>{risk}</li>)}</ul><p>Latest source revision: {info.latestCommit?.slice(0, 12)}. Local files are not updated automatically.</p></>}
    {capability.origin === 'discovery' && <><button className="pod-secondary" disabled={!desktop || busy} onClick={() => void inspect()}>Inspect current source</button><button className="pod-secondary" disabled={!desktop || busy} onClick={() => void preview()}>Preview installation</button></>}
    <p role="status">{message}</p>
    <details><summary>Edit functional classification</summary>
      <label className="console-field"><span>Primary function</span><select aria-label="Primary function" value={primary} onChange={(event) => setPrimary(event.target.value)}><option value="">Pending review</option>{discoveryCategories.map((category) => <option key={category.id} value={category.id}>{category.label}</option>)}</select></label>
      <label className="console-field"><span>Related functions</span><select multiple aria-label="Related functions" value={related} onChange={(event) => setRelated([...event.target.selectedOptions].map((option) => option.value))}>{discoveryCategories.map((category) => <option key={category.id} value={category.id}>{category.label}</option>)}</select></label>
      <button className="pod-secondary" disabled={!desktop || busy} onClick={() => void saveCategory()}>Save functional classification</button>
    </details>
    {plan && <div aria-label="Installation preview"><h2>Installation preview</h2><p>Dry run only. No download, install, enable, or configuration write has occurred.</p><dl><div><dt>Source</dt><dd>{plan.sourceRepository}</dd></div><div><dt>Revision</dt><dd>{plan.commit ?? 'Unverified'}</dd></div><div><dt>Target</dt><dd>{plan.targetDirectory}</dd></div><div><dt>Local state</dt><dd>{plan.localState}</dd></div></dl><ul>{[...plan.blockers, ...plan.warnings, ...plan.risks ?? []].map((text) => <li key={text}>{text}</li>)}</ul><p>Rollback: a verified backup must be created before any Phase 6 writes.</p><details><summary>{plan.writePreview.length} proposed file writes</summary><ul>{plan.writePreview.map((file) => <li key={file.targetPath}>{file.targetPath}{file.conflict ? ' · conflict' : ''}</li>)}</ul></details>{plan.dependencies.map((dependency) => <p key={dependency.command}>{dependency.command}: {dependency.foundOnPath ? 'found on PATH' : 'setup required'}</p>)}</div>}
  </section></>
}
