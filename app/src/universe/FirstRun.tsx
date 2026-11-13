import { useEffect, useRef, useState } from 'react'
import { Box, GitBranch, Orbit, Settings2 } from 'lucide-react'
import './first-run.css'
import { saveWelcome } from './firstRunState'

const steps = [
  { title: 'Map your local Codex', text: 'Scan the Skills, plugins and MCP configuration on this machine. The scan reads metadata and leaves configuration untouched.', action: 'Open Installed', target: 'installed', Icon: Box },
  { title: 'Add a provider when you need one', text: 'The universe works offline. OpenAI and DeepSeek are optional and share a US$1 monthly API cap. A key stays in the Windows credential store.', action: 'Open AI Providers', target: 'providers', Icon: Settings2 },
  { title: 'Review a mission before it runs', text: 'Explore capabilities in World Tree, then assemble a plan in Mission Control. The app shows the exact execution scope before approval.', action: 'Open Mission Control', target: 'mission', Icon: GitBranch },
] as const
export type FirstRunTarget = typeof steps[number]['target'] | 'world'

export default function FirstRun({ initialStep, codexDetected, onClose }: {
  initialStep: number; codexDetected: boolean | null; onClose: (target?: FirstRunTarget) => void
}) {
  const dialog = useRef<HTMLDialogElement>(null)
  const [step, setStep] = useState(initialStep)
  const current = steps[step]
  useEffect(() => {
    const element = dialog.current
    element?.showModal()
    return () => element?.close()
  }, [])
  useEffect(() => saveWelcome(step, false), [step])
  const close = (target?: FirstRunTarget) => {
    saveWelcome(step, true)
    onClose(target)
  }
  return <dialog className="first-run" ref={dialog} aria-labelledby="first-run-title" onCancel={event => { event.preventDefault(); close() }}>
    <div className="first-run-orbit" aria-hidden="true"><Orbit size={42}/></div>
    <div className="first-run-content">
      <p className="first-run-brand">SkillVerse AI</p>
      <h1 id="first-run-title">Your capability universe</h1>
      <p className="first-run-status">{codexDetected === null ? 'Codex status unavailable' : codexDetected ? 'Codex CLI found on PATH' : 'Codex CLI absent on PATH · offline tools available'}</p>
      <div className="first-run-step">
        <span className="first-run-position"><current.Icon size={16}/> {step + 1} of {steps.length}</span>
        <h2>{current.title}</h2>
        <p>{current.text}</p>
        <button type="button" className="first-run-link" onClick={() => close(current.target)}>{current.action}</button>
      </div>
      <div className="first-run-actions">
        <button type="button" onClick={() => close()}>Later</button>
        {step > 0 && <button type="button" onClick={() => setStep(step - 1)}>Back</button>}
        <button type="button" className="first-run-primary" onClick={() => step + 1 < steps.length ? setStep(step + 1) : close('world')}>
          {step + 1 < steps.length ? 'Next' : 'Explore World Tree'}
        </button>
      </div>
    </div>
  </dialog>
}
