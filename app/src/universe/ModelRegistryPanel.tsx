import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import './projectRouting.css'

export type ModelScope = { modelId: string; allowNetwork: boolean; maxTokens: number; maxCostMicrousd: number; maxAttempts: number }
type Model = { config: { id: string; modelId: string; providerId: string }; transport: string; revision: string; cooldownUntil?: number; health: { source: string; observed: string[]; lastError?: string } }
type Budget = { monthlyLimit: number; dailyLimit: number; taskLimit: number; acceptanceLimit: number; monthRemaining: number; dayRemaining: number; acceptanceCommitted: number; settledMonth: number; unsettled: number }
type Registry = { ready: boolean; schema: number; budget?: Budget; models: Model[]; providers: Array<{ id: string; name: string; credentialPresent?: boolean }>; history: Array<{ id: string; modelId: string; status: string; attempts: number; costUpperBoundMicrousd?: number; error?: string }>; suggestedSimple?: { modelId?: string; reason: string }; suggestedComplex?: { modelId?: string; reason: string } }
const call = <T,>(action: string, args: unknown = {}) => invoke<T>('capability_intelligence', { action, args })
const usd = (amount: number) => '$' + (amount / 1000000).toFixed(amount % 10000 === 0 ? 2 : 6)
const template = JSON.stringify({
  provider: { id: 'offline', name: 'Offline mock — no model service', transport: 'mock', baseUrl: '', enabled: true },
  model: { id: 'offline-routing', providerId: 'offline', modelId: 'offline-fixture-v1', enabled: true,
    contextWindow: 32768, maxOutputTokens: 1024, timeoutSeconds: 10, maxAttempts: 1,
    dailyRequests: 100, dailyTokens: 1000000, dailyBudgetMicrousd: 0,
    taskTokens: 20000, taskBudgetMicrousd: 0,
    inputPricePerMillionMicrousd: 0, outputPricePerMillionMicrousd: 0,
    declaredFeatures: ['text', 'structured_output', 'tool_selection', 'reasoning'] },
}, null, 2)

export default function ModelRegistryPanel({ disabled, complex, onSelection, settings = false }: { disabled: boolean; complex: boolean; onSelection: (scope: ModelScope | undefined) => void; settings?: boolean }) {
  const [open, setOpen] = useState(settings)
  const [registry, setRegistry] = useState<Registry>()
  const [modelId, setModelId] = useState('')
  const [config, setConfig] = useState(template)
  const [tokens, setTokens] = useState(20000)
  const [cost, setCost] = useState(0)
  const [attempts, setAttempts] = useState(1)
  const [consent, setConsent] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [notice, setNotice] = useState('')
  const [remove, setRemove] = useState(false)
  const [clock, setClock] = useState(Date.now)
  const key = useRef<HTMLInputElement>(null)
  const automatic = complex ? registry?.suggestedComplex : registry?.suggestedSimple
  const resolvedId = modelId === 'auto' ? automatic?.modelId : modelId
  const selected = registry?.models.find(m => m.config.id === resolvedId)
  const authorization = JSON.stringify([resolvedId, selected?.revision, tokens, cost, attempts])
  const mock = selected?.transport === 'mock'
  const paid = selected?.config.providerId === 'phase13-openai' || selected?.config.providerId === 'phase13-deepseek'
  const credential = registry?.providers.find(provider => provider.id === selected?.config.providerId)?.credentialPresent
  const cooling = paid && (selected?.cooldownUntil ?? 0) > clock
  const allowed = !!selected && (mock || consent === authorization)
  const healthy = !!selected && (!paid || (credential && !cooling && selected.health.source === 'LIVE_PROBE_VERIFIED'))
    && ['text', 'structured_output', 'tool_selection'].every(feature => selected.health.observed.includes(feature))

  useEffect(() => {
    if (!open) return
    let active = true
    void call<Registry>('model-status').then(value => { if (active) { setRegistry(value); setClock(Date.now()) } })
      .catch(reason => { if (active) setError(typeof reason === 'string' ? reason : 'Could not read model settings.') })
    return () => { active = false }
  }, [open])
  useEffect(() => {
    if (!open || !cooling) return
    const delay = Math.min((selected?.cooldownUntil ?? 0) - clock, 30_000)
    const timer = setTimeout(() => setClock(Date.now()), delay + 10)
    return () => clearTimeout(timer)
  }, [clock, cooling, open, selected?.cooldownUntil])
  useEffect(() => {
    onSelection(allowed && healthy && resolvedId ? { modelId: resolvedId, allowNetwork: !mock, maxTokens: tokens, maxCostMicrousd: cost, maxAttempts: attempts } : undefined)
  }, [allowed, healthy, resolvedId, mock, tokens, cost, attempts, onSelection])
  useEffect(() => { if (key.current) key.current.value = '' }, [modelId])
  const reload = async () => { setRegistry(await call<Registry>('model-status')); setClock(Date.now()) }
  const work = async (perform: () => Promise<void>) => {
    setBusy(true); setError(''); setNotice('')
    try { await perform(); await reload() }
    catch (reason) {
      setError(typeof reason === 'string' ? reason : 'Model operation failed. Saved history was retained.')
      await reload().catch(() => undefined)
    }
    finally { setBusy(false) }
  }
  const locked = disabled || busy
  return <details className="route-models" open={open} onToggle={event => setOpen(event.currentTarget.open)}>
    <summary>{settings ? 'Provider connections and spending' : 'Task planning models'} <span>{selected ? (mock ? 'Offline mock' : selected.config.modelId) : 'No model selected'}</span></summary>
    <p>Models propose plans. Execution requires separate approval in Mission Control.</p>
    {registry && settings && registry.schema < 13 && <>
      <p>Enable OpenAI and DeepSeek with a verified backup and independent restore before each database migration.</p>
      <button disabled={locked || registry.schema < 10} onClick={() => void work(async () => {
        if (registry.schema === 10) await call('routing-initialize', { confirmed: true })
        if (registry.schema <= 11) await call('model-initialize', { confirmed: true })
        await call('model-live-initialize', { confirmed: true })
        setNotice('Live model slots enabled. Add a dedicated provider key and check a model to verify access.')
      })}>Enable live AI providers</button>
    </>}
    {registry && !registry.ready && !settings && <>
      <p>Enable the model registry with a verified backup and independent database restore.</p>
      <button disabled={locked || registry.schema !== 11} onClick={() => void work(async () => {
        await call('model-initialize', { confirmed: true }); setNotice('Model registry enabled.')
      })}>Enable model registry</button>
    </>}
    {registry?.ready && <>
      {registry.budget && <section className="route-live-budget" aria-label="Shared live AI budget">
        <strong>{usd(registry.budget.monthRemaining)} remaining this month</strong>
        <progress aria-label="Monthly committed model budget" max={registry.budget.monthlyLimit} value={registry.budget.monthlyLimit - registry.budget.monthRemaining}/>
        <p>{usd(registry.budget.settledMonth)} estimated from reported usage · {usd(registry.budget.unsettled)} reserved without final usage</p>
        <p>{usd(registry.budget.dayRemaining)} of {usd(registry.budget.dailyLimit)} left today (UTC) · {usd(registry.budget.taskLimit)} per task · {usd(registry.budget.acceptanceCommitted)} of {usd(registry.budget.acceptanceLimit)} committed to acceptance</p>
        {registry.budget.dayRemaining <= registry.budget.dailyLimit / 5 && <p className="route-budget-warning" role="status">Daily live AI budget is low. Offline planning is available.</p>}
        {registry.budget.unsettled > 0 && <p>Unknown provider charges stay reserved until verified.</p>}
      </section>}
      {settings && registry.budget && <ul className="route-list" aria-label="Provider connection status">
        {registry.providers.filter(provider => provider.id.startsWith('phase13-')).map(provider => <li key={provider.id}>
          <strong>{provider.name}</strong>
          <span>{provider.credentialPresent ? 'Dedicated key saved' : 'Key required'}</span>
          <small>{registry.models.filter(model => model.config.providerId === provider.id && model.health.source === 'LIVE_PROBE_VERIFIED' && provider.credentialPresent).length} / 2 models verified</small>
        </li>)}
      </ul>}
      <label>Planning model<select aria-label="Planning model" disabled={locked} value={modelId} onChange={event => {
        const nextId = event.target.value
        const next = registry.models.find(model => model.config.id === (nextId === 'auto' ? automatic?.modelId : nextId))
        setModelId(nextId); setConsent(''); setRemove(false); setAttempts(1)
        setCost(next?.config.providerId.startsWith('phase13-') ? (registry.budget?.taskLimit ?? 20000) : 0)
      }}>
        <option value="">Choose a configured model</option>
        <option value="auto">Auto · observed features and configured cost</option>
        {registry.models.map(m => <option key={m.config.id} value={m.config.id}>{m.config.modelId}{m.transport === 'mock' ? ' · OFFLINE MOCK' : ''}</option>)}
      </select></label>
      <p>{automatic?.modelId ? `Suggested: ${automatic.modelId}. ${automatic.reason}` : 'Check a configured model before automatic model selection is available.'}</p>
      {selected && <>
        <p>{mock ? 'Offline mock' : paid ? (credential ? (healthy ? 'Live model verified' : 'Dedicated key saved · live model unverified') : 'Dedicated key required') : 'Local adapter'} · Observed features {selected.health.observed.join(', ') || 'none'}.</p>
        <p>{selected.health.source || 'Awaiting connection check'}{selected.health.lastError ? ' · ' + selected.health.lastError : ''}. Embedding and long context require separate tests.</p>
        {cooling && <p role="status" className="route-budget-warning">Provider cooling down until {new Date(selected.cooldownUntil!).toLocaleTimeString()}. Choose a different verified provider or use offline planning.</p>}
        <div className="route-model-limits">
          <label>Request token limit<input aria-label="Request token limit" type="number" min={64} max={2000000} value={tokens} disabled={locked} onChange={event => setTokens(Number(event.target.value))}/></label>
          <label>Request cost limit (USD)<input aria-label="Request cost limit in USD" type="number" min={0} max={paid ? .02 : 1000} step={0.001} value={cost / 1000000} disabled={locked} onChange={event => setCost(Math.round(Number(event.target.value) * 1000000))}/></label>
          {!paid && <label>Maximum attempts<input aria-label="Maximum model attempts" type="number" min={1} max={2} value={attempts} disabled={locked} onChange={event => setAttempts(Number(event.target.value))}/></label>}
        </div>
        {!mock && <label className="route-model-consent"><input type="checkbox" disabled={locked} checked={consent === authorization} onChange={event => setConsent(event.target.checked ? authorization : '')}/>Allow this provider to receive the redacted task and bounded capability metadata within these limits</label>}
        <button disabled={locked || !allowed || cooling || (paid && !credential)} onClick={() => void work(async () => {
          await call('model-health', { confirmed: true, scope: { requestId: crypto.randomUUID(), modelId: resolvedId, allowNetwork: !mock, maxTokens: tokens, maxCostMicrousd: cost, maxAttempts: attempts } })
          setNotice(mock ? 'Offline mock probe passed. Live model not verified.' : 'Model check finished. See probe evidence and request history.')
        })}>Check selected model</button>
        {paid && <details><summary>Secure API key</summary>
          <label>API key<input aria-label="Model API key" ref={key} type="password" autoComplete="new-password" disabled={locked}/></label>
          <button disabled={locked} onClick={() => void work(async () => {
            const value = key.current?.value ?? ''; if (key.current) key.current.value = ''
            await call('model-credential', { confirmed: true, providerId: selected.config.providerId, value })
            setNotice('Key saved in the operating system credential store.')
          })}>Store key securely</button>
          {credential && <><label className="route-model-consent"><input type="checkbox" checked={remove} onChange={event => setRemove(event.target.checked)}/>Remove this app’s saved key</label>
            <button disabled={locked || !remove} onClick={() => void work(async () => {
              await call('model-credential', { confirmed: true, providerId: selected.config.providerId, remove: true })
              setRemove(false); setNotice('Local key removed. Use the official provider console to revoke the remote key.')
            })}>Remove local key</button></>}
        </details>}
      </>}
      <details><summary>Configure a provider and model</summary>
        <p>Use this form for offline fixtures and local loopback adapters. Live HTTPS access uses the managed OpenAI and DeepSeek slots. Save keys under Secure API key.</p>
        <label>Provider and model configuration<textarea aria-label="Provider and model configuration" spellCheck={false} maxLength={8192} value={config} disabled={locked} onChange={event => setConfig(event.target.value)}/></label>
        <button disabled={locked} onClick={() => void work(async () => {
          const draft = JSON.parse(config) as { provider: { id: string }; model: { id: string } }
          await call('model-save-provider', { confirmed: true, provider: draft.provider })
          await call('model-save-model', { confirmed: true, model: draft.model })
          setModelId(draft.model.id); setConsent(''); setNotice('Configuration saved. Check the selected model before generation.')
        })}>Save provider and model</button>
      </details>
      {!!registry.history.length && <details><summary>Model request history</summary>
        <ul className="route-list">{registry.history.map(row => <li key={row.id}>
          <strong>{row.modelId} · {row.status.toLowerCase()}</strong><span>{row.attempts} attempts · cost upper bound {row.costUpperBoundMicrousd == null ? 'unknown' : '$' + (row.costUpperBoundMicrousd / 1000000).toFixed(6)}</span>{row.error && <small>{row.error}</small>}
        </li>)}</ul>
        <p>Daily budgets use UTC. Unknown usage keeps its reservation. Configured prices do not prove actual billing.</p>
      </details>}
      <button disabled={locked} onClick={() => void work(reload)}>Refresh model status</button>
    </>}
    {busy && <p role="status">Checking model settings…</p>}
    {error && <p role="alert" className="mission-error">{error}</p>}
    {notice && <p role="status">{notice}</p>}
  </details>
}
