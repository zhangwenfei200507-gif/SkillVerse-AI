import { useEffect, useState } from 'react'
import { invoke, isTauri } from '@tauri-apps/api/core'
import { ArrowUpRight, Compass, LoaderCircle, ShieldCheck } from 'lucide-react'
import './projectRouting.css'
import ModelRegistryPanel, { type ModelScope } from './ModelRegistryPanel'

type Entry = { score: number; reasons: string[]; candidate: {
  id: string; name: string; kind: string; availability: string; missing: string[]
} }
type Proposal = {
  id: string
  context: { name: string; frameworks: string[]; languages: string[]; testTools: string[] }
  intents: string[]
  complexity: string
  ranked: Entry[]
  selected: string[]
  related: Array<{ left: string; right: string; kind: string; state: string; reason: string }>
  missing: string[]
  workflowPreview: string[]
  reviewedForDraft: unknown[]
  cachedContext: boolean
  semantic?: { mode: string; status: string; confidence?: number; receipt?: { provider?: string; actualModelId?: string; estimatedTokens?: number; estimatedMicrousd?: number; settledMicrousd?: number; cacheHit?: boolean } }
  task?: { goalSummary: string; operations: string[]; riskLevel: string; acceptanceCriteria: string[] }
}
type Status = {
  ready: boolean; schema: number
  history: Array<{ id: string; taskType: string; createdAt: number; workflowId?: string }>
  provider: { provider: string; model: string; connection: string; dailyRequests: number; dailyTokens: number; lastError?: string }
}
const call = <T,>(action: string, args: unknown = {}) => invoke<T>('capability_intelligence', { action, args })
const label = (value: string) => value.toLowerCase().replaceAll('_', ' ')
const estimatedUsd = (value?: number) => value == null ? 'unavailable' : '$' + (value / 1000000).toFixed(6)
const errorText = (value: unknown) => typeof value === 'string' ? value : 'Project routing could not finish. Your draft and saved data were retained.'

export default function ProjectRoutingPanel({ goal, project, disabled, onHandoff }: {
  goal: string; project: string; disabled: boolean; onHandoff: (id: string) => void
}) {
  const [open, setOpen] = useState(false)
  const [status, setStatus] = useState<Status>()
  const [proposal, setProposal] = useState<Proposal>()
  const [consent, setConsent] = useState('')
  const [analysisInput, setAnalysisInput] = useState('')
  const [busy, setBusy] = useState('')
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [rating, setRating] = useState('HELPFUL')
  const [failure, setFailure] = useState('NONE')
  const [chosen, setChosen] = useState('')
  const [modelScope, setModelScope] = useState<ModelScope>()
  const [requestId, setRequestId] = useState('')
  const [replacements, setReplacements] = useState<Record<string, string>>({})
  const desktop = isTauri()
  const inputKey = JSON.stringify([goal.trim(), project.trim()])
  const authorized = !!project.trim() && consent === project.trim()
  const locked = disabled || !!busy

  useEffect(() => {
    if (!open || !desktop) return
    let current = true
    void call<Status>('routing-status')
      .then(result => { if (current) setStatus(result) })
      .catch(reason => { if (current) setError(errorText(reason)) })
    return () => { current = false }
  }, [desktop, open])

  async function work(activity: string, perform: () => Promise<void>) {
    setBusy(activity)
    setError('')
    setNotice('')
    try { await perform() }
    catch (reason) { setError(errorText(reason)) }
    finally { setBusy('') }
  }

  function analyze(useModel = false) {
    void work('Analyzing project metadata and capability evidence…', async () => {
      const id = useModel ? crypto.randomUUID() : ''
      setRequestId(id)
      let result: Proposal
      try {
        result = await call<Proposal>('routing-analyze', { goal, projectRoot: project, confirmed: authorized,
          ...(useModel && modelScope ? { modelRequest: { ...modelScope, requestId: id } } : {}) })
      } finally { setRequestId('') }
      setProposal(result)
      setReplacements({})
      setChosen(result.selected[0] ?? '')
      setAnalysisInput(inputKey)
      setStatus(await call<Status>('routing-status'))
    })
  }

  const selected = proposal?.selected.flatMap(id => proposal.ranked.filter(row => row.candidate.id === id)) ?? []
  const alternate = proposal?.ranked.filter(row => !proposal.selected.includes(row.candidate.id)) ?? []
  const candidateName = (id: string) => proposal?.ranked.find(row => row.candidate.id === id)?.candidate.name ?? id
  return (
    <details className="project-routing" open={open} onToggle={event => setOpen(event.currentTarget.open)}>
      <summary><Compass size={17} aria-hidden="true"/> Project Intelligence <span>Analyze before planning</span></summary>
      <p>Match the goal above against a read-only project snapshot and your capability catalog.</p>
      {!desktop && <p>Open the desktop app to analyze a local project.</p>}
      {desktop && status && !status.ready && <div className="route-enable">
        <p>Enable project routing history. SkillVerse verifies a backup and independent restore before upgrading the database.</p>
        <button disabled={locked || status.schema !== 10} onClick={() => void work('Verifying backup and enabling routing…', async () => {
          await call('routing-initialize', { confirmed: true })
          setStatus(await call<Status>('routing-status'))
          setNotice('Routing history is ready.')
        })}>Enable routing history</button>
        {status.schema !== 10 && <p>Initialize Mission Control storage first.</p>}
      </div>}
      {status?.ready && <>
        <div className="route-controls">
          <label>
            <input type="checkbox" checked={authorized} disabled={locked || !project.trim()}
              onChange={event => setConsent(event.target.checked ? project.trim() : '')}/>
            Allow read-only metadata analysis for the selected project directory
          </label>
          <button className="route-action" disabled={locked || !authorized || !goal.trim()} onClick={() => analyze()}>
            <Compass size={15}/> Analyze task & project
          </button>
          <button className="route-action" disabled={locked || !authorized || !goal.trim() || !modelScope} onClick={() => analyze(true)}>Generate with AI</button>
          {requestId && <button onClick={() => void call('model-cancel', { requestId }).catch(reason => setError(errorText(reason)))}>Cancel model request</button>}
        </div>
        <details className="route-provider">
          <summary>Source analysis model and budget</summary>
          <p>{label(status.provider.connection)} · offline rules are available for every project.</p>
          <dl>
            <div><dt>Provider / model</dt><dd>{status.provider.provider} / {status.provider.model || 'Unspecified'}</dd></div>
            <div><dt>Daily limit</dt><dd>{status.provider.dailyRequests} requests · {status.provider.dailyTokens} tokens</dd></div>
            <div><dt>Cost</dt><dd>Unknown until a provider and budget are authorized</dd></div>
            <div><dt>Latest provider error</dt><dd>{status.provider.lastError || 'None recorded'}</dd></div>
          </dl>
        </details>
        <ModelRegistryPanel disabled={locked} complex={proposal?.complexity === 'COMPLEX'} onSelection={setModelScope}/>
        {status.history.length > 0 && <label className="route-history">Analysis history
          <select aria-label="Routing history" value={proposal?.id ?? ''} disabled={locked} onChange={event => {
            const decisionId = event.target.value
            if (!decisionId) return
            void work('Reading saved analysis…', async () => {
              const saved = await call<Proposal>('routing-get', { decisionId })
              setProposal(saved)
              setChosen(saved.selected[0] ?? '')
              setAnalysisInput('')
            })
          }}>
            <option value="">Select a saved analysis</option>
            {status.history.map(item => <option key={item.id} value={item.id}>
              {label(item.taskType)} · {new Date(item.createdAt).toLocaleString()}{item.workflowId ? ' · draft saved' : ''}
            </option>)}
          </select>
        </label>}
      </>}
      {busy && <div className="route-progress" role="status" data-planning={!!requestId}>
        <LoaderCircle size={16} className="mission-spinner"/>
        <span>{busy}{requestId && <small>Checking the model response against the reviewed capability graph.</small>}</span>
      </div>}
      {error && <p role="alert" className="mission-error">{error}</p>}
      {notice && <p role="status" className="route-notice">{notice}</p>}
      {proposal && <section className="route-workbench" aria-label="Project routing proposal">
        <header className="route-project">
          <div>
            <h3>{proposal.context.name}</h3>
            <p>{[...proposal.context.frameworks, ...proposal.context.languages, ...proposal.context.testTools].join(' · ') || 'Project stack undetermined'}</p>
          </div>
          <span>{label(proposal.complexity)} task · {proposal.cachedContext ? 'cached snapshot' : 'fresh snapshot'}</span>
        </header>
        {proposal.semantic && <p>Decision: {label(proposal.semantic.mode)} · {label(proposal.semantic.status)}{proposal.semantic.confidence == null ? '' : ` · ${Math.round(proposal.semantic.confidence * 100)}% reported confidence`}</p>}
        {proposal.semantic?.receipt?.provider?.startsWith('phase13-') && <div className="route-live-budget" aria-label="Model usage receipt">
          <strong>{proposal.semantic.receipt.actualModelId}</strong>
          <p>{proposal.semantic.receipt.cacheHit ? 'Cached response · no network request' : `${proposal.semantic.receipt.estimatedTokens ?? '—'} estimated tokens · ${estimatedUsd(proposal.semantic.receipt.estimatedMicrousd)} reserved`}</p>
          <p>{estimatedUsd(proposal.semantic.receipt.settledMicrousd)} estimated from provider usage. Execution approval is separate.</p>
        </div>}
        {proposal.task && <details><summary>Task understanding and acceptance</summary>
          <p>{proposal.task.goalSummary}</p><p>{proposal.task.operations.join(' → ')} · {label(proposal.task.riskLevel)} risk · execution approval required</p>
          <ul className="route-list">{proposal.task.acceptanceCriteria.map(item => <li key={item}>{item}</li>)}</ul>
        </details>}
        <div className="route-columns">
          <section aria-label="Recommended capabilities">
            <h4>Recommended route</h4>
            <p className="route-intents">{proposal.intents.map(label).join(' / ')}</p>
            {selected.length === 0 && <p>The bounded local report can start with project metadata. Add a more specific goal to match an external capability.</p>}
            {selected.map(({ candidate, score, reasons }) => <article className="route-match" key={candidate.id}>
              <div className="route-match-head"><strong>{candidate.name}</strong><span>{candidate.kind} · {label(candidate.availability)}</span></div>
              <div className="route-score">
                <meter aria-label={candidate.name + ' evidence match'} min={0} max={100} value={score}/>
                <span>{score}/100 match</span>
              </div>
              <p>{reasons.join(' · ')}</p>
              <small>{candidate.missing.join(' · ')}</small>
              <label>Replace {candidate.name}<select aria-label={'Replace capability ' + candidate.name} value={replacements[candidate.id] ?? candidate.id} disabled={locked || analysisInput !== inputKey}
                onChange={event => setReplacements(previous => ({ ...previous, [candidate.id]: event.target.value }))}>
                {proposal.ranked.map(row => <option key={row.candidate.id} value={row.candidate.id} disabled={['DISABLED', 'UNTRUSTED_METADATA', 'REVIEW_BLOCKED'].includes(row.candidate.availability)}>{row.candidate.name}</option>)}
              </select></label>
            </article>)}
            {!!selected.length && <button disabled={locked || !authorized || analysisInput !== inputKey || Object.keys(replacements).length === 0}
              onClick={() => void work('Validating capability choices…', async () => {
                const next = await call<Proposal>('routing-replace', { decisionId: proposal.id, goal, projectRoot: project, confirmed: authorized,
                  selected: proposal.selected.map(id => replacements[id] ?? id) })
                setProposal(next); setReplacements({}); setStatus(await call<Status>('routing-status'))
                setNotice('Choices saved as a new proposal. Existing workflows were retained.')
              })}>Save capability choices</button>}
            {!!alternate.length && <details><summary>Alternatives and blocked candidates ({alternate.length})</summary>
              <ul className="route-list">{alternate.map(({candidate,score}) => <li key={candidate.id}>
                <strong>{candidate.name}</strong><span>{score}/100 · {label(candidate.availability)}</span>
                <small>{candidate.missing.join(' · ')}</small>
              </li>)}</ul>
            </details>}
          </section>
          <section aria-label="Proposed workflow">
            <h4>Mission Control preview</h4>
            <ol className="route-sequence">{proposal.workflowPreview.map((step, index) => <li key={index + ':' + step}>{step}</li>)}</ol>
            <p className="route-approval"><ShieldCheck size={16}/> {proposal.reviewedForDraft.length ? 'Reviewed sources require separate runtime checks and approval.' : 'The draft uses local report steps. Unavailable candidates remain recommendations.'}</p>
            {!!proposal.missing.length && <ul className="route-constraints">{proposal.missing.map(item => <li key={item}>{item}</li>)}</ul>}
            {!!proposal.related.length && <details><summary>Source relationships</summary>
              <ul className="route-list">{proposal.related.map(relation => <li key={[relation.left,relation.right,relation.kind].join(':')}>
                <strong>{candidateName(relation.left)} / {candidateName(relation.right)}</strong>
                <span>{label(relation.kind)} · {label(relation.state)}</span><small>{relation.reason}</small>
              </li>)}</ul>
            </details>}
            <button className="route-action" disabled={locked || !authorized || analysisInput !== inputKey}
              onClick={() => void work('Saving an editable Mission Control draft…', async () => {
                const result = await call<{workflowId: string}>('routing-handoff', {
                  decisionId: proposal.id, goal, projectRoot: project, confirmed: authorized,
                })
                onHandoff(result.workflowId)
                setNotice('Draft saved. Review its scope in Mission Control.')
                setStatus(await call<Status>('routing-status'))
              })}>Open editable draft <ArrowUpRight size={16}/></button>
            <button className="route-action" disabled={locked || !authorized || !goal.trim()}
              onClick={() => void work('Preparing a first-party Codex draft…', async () => {
                const result = await call<{workflowId: string}>('routing-handoff', {
                  decisionId: proposal.id, goal, projectRoot: project, confirmed: authorized, firstPartyOnly: true,
                })
                onHandoff(result.workflowId)
                setNotice('Review the sealed Codex fixture and local report scope in Mission Control.')
                setStatus(await call<Status>('routing-status'))
              })}>Prepare first-party Codex draft <ShieldCheck size={16}/></button>
            <p>Reads a sealed fixture through trusted Codex. Recommended Skills, plugins and MCP servers remain advisory in this draft.</p>
            {analysisInput !== inputKey && <p>To reuse saved advice for the first-party draft, enter the original goal and select its project. The app rechecks both before handoff.</p>}
          </section>
        </div>
        <details className="route-feedback"><summary>Rate this recommendation</summary>
          <div>
            <label>Recommendation rating<select value={rating} onChange={e=>setRating(e.target.value)}>
              {['HELPFUL','INACCURATE','UNAVAILABLE','BETTER_ALTERNATIVE','TOO_COMPLEX'].map(item=><option key={item} value={item}>{label(item)}</option>)}
            </select></label>
            <label>Failure category<select value={failure} onChange={e=>setFailure(e.target.value)}>
              {['NONE','TOOL','SETUP','NETWORK','SELECTION','CANCELLED','DENIED','UNKNOWN'].map(item=><option key={item} value={item}>{label(item)}</option>)}
            </select></label>
            <label>Capability selected<select value={chosen} onChange={e=>setChosen(e.target.value)}>
              <option value="">No capability</option>
              {proposal.ranked.map(row=><option key={row.candidate.id} value={row.candidate.id}>{row.candidate.name}</option>)}
            </select></label>
            <button disabled={locked} onClick={()=>void work('Saving local feedback…',async()=>{
              await call('routing-feedback',{decisionId:proposal.id,rating,failureClass:failure,chosenId:chosen || undefined})
              setNotice('Feedback saved locally.')
            })}>Save feedback</button>
          </div>
        </details>
      </section>}
    </details>
  )
}
