import {useEffect,useState} from 'react'
import {invoke,isTauri} from '@tauri-apps/api/core'
export type ReviewedSelection={id:string;digest:string}
type ReviewedCapabilityScope = {
 profileId:string; name:string; kind:string; repository:string; commit:string;
 process:string; network:string; account:string; writes:string; expectedOutput:string;
 risk:string; estimatedCost:string; tool?:string; endpoint?:string; readFiles?:string[]
}
export function ReviewedScope({scope}:{scope:unknown}){
 const entries=(scope as {reviewedCapabilities?:ReviewedCapabilityScope[]}|null)?.reviewedCapabilities
 if(!entries?.some(item=>item.name))return null
 return <div className="mission-reviewed-scope" aria-label="Reviewed capability scope">
  {entries.filter(item=>item.name).map(item=><details key={item.profileId}>
   <summary><strong>{item.name}</strong><span>{item.kind} · {item.risk}</span></summary>
   <dl className="mission-facts">
    <div><dt>Source and revision</dt><dd>{item.repository}<code>{item.commit}</code></dd></div>
    <div><dt>Execution</dt><dd>{item.process}{item.tool&&<code>{item.tool}</code>}</dd></div>
    <div><dt>Network and account</dt><dd>{item.endpoint||item.network}<br/>{item.account}</dd></div>
    <div><dt>Reads and writes</dt><dd>{item.readFiles?.length??0} reviewed package files<br/>{item.writes}</dd></div>
    <div><dt>Expected output</dt><dd>{item.expectedOutput}</dd></div>
    <div><dt>Estimated cost</dt><dd>{item.estimatedCost}</dd></div>
   </dl>
  </details>)}
 </div>
}
type Profile={id:string;digest:string;name:string;kind:string;mode:string;policyReady:boolean;enabled:boolean;blockers:string[];transport:string;repository:string;commit:string;bundle?:{structure?:string;bundledSkills?:string[];hasMcp?:boolean;hasApps?:boolean;hasHooks?:boolean}}
export default function CompatibilityPanel({selected,onChange,disabled=false}:{selected:ReviewedSelection[];onChange:(items:ReviewedSelection[])=>void;disabled?:boolean}){
 const [profiles,setProfiles]=useState<Profile[]>([]),[error,setError]=useState(''),[busy,setBusy]=useState(false),[kind,setKind]=useState('skill'),[root,setRoot]=useState(''),[repository,setRepository]=useState(''),[commit,setCommit]=useState(''),[component,setComponent]=useState('skills/example/SKILL.md')
 const [networkApproved,setNetworkApproved]=useState(false)
 const refresh=async()=>{const result=await invoke<{profiles:Profile[]}>('workflow_engine',{action:'compatibility-list',args:{}});setProfiles(Array.isArray(result.profiles)?result.profiles:[])}
 useEffect(()=>{let alive=true;if(isTauri())void invoke<{profiles:Profile[]}>('workflow_engine',{action:'compatibility-list',args:{}}).then(result=>{if(alive)setProfiles(Array.isArray(result.profiles)?result.profiles:[])}).catch(()=>{if(alive)setError('Reviewed sources could not be loaded. Existing plans were kept.')});return()=>{alive=false}},[])
 const review=async()=>{setBusy(true);setError('');try{await invoke('workflow_engine',{action:'review-capability',args:kind==='mcp'?{kind,networkApproved,name:'OpenAI Docs public search',repository:'https://developers.openai.com/learn/docs-mcp',commit:'NOT_PUBLISHED_HOSTED_SERVICE',endpoint:'https://developers.openai.com/mcp',tool:'search_openai_docs'}:{kind,root,repository,commit,component,mode:'INSTRUCTION_ONLY'}});await refresh();setNetworkApproved(false)}catch(e){setError(typeof e==='string'?e:'Source review failed. No runtime was started.')}finally{setBusy(false)}}
 const toggle=(p:Profile)=>onChange(selected.some(x=>x.id===p.id)?selected.filter(x=>x.id!==p.id):[...selected,{id:p.id,digest:p.digest}].slice(0,3))
 return <details className="mission-compatibility"><summary>Reviewed third-party sources{selected.length?` · ${selected.length} selected`:''}</summary><p>Select up to three sources in execution order. Review allows preparation only; a run still requires its exact scope approval. Runtime and task results are verified separately.</p><div className="compatibility-sources">{profiles.map(p=><label key={p.id} className={'compatibility-source '+(p.policyReady?'source-ready':'source-blocked')}><input type="checkbox" aria-label={'Use reviewed '+p.name} checked={selected.some(x=>x.id===p.id)} disabled={disabled||!p.policyReady||(!selected.some(x=>x.id===p.id)&&selected.length>=3)} onChange={()=>toggle(p)}/><span><strong>{p.name}</strong><small>{p.kind} · {p.transport} · {p.mode}</small><small>{p.enabled?(p.policyReady?'Policy reviewed · runtime not yet verified':p.blockers.join('; ')):'Disabled · calls blocked'}</small>{p.bundle?.structure&&<small>{[
  (p.bundle.bundledSkills?.length??0)+' bundled Skills',
  p.bundle.hasMcp?'MCP declared':null,
  p.bundle.hasApps?'App declared':null,
  p.bundle.hasHooks?'Hooks require review':null,
].filter(Boolean).join(' · ')}</small>}<code title={p.commit}>{p.repository} · {p.commit}</code></span></label>)}</div>{!profiles.length&&<p>No reviewed sources yet. Existing installed capabilities stay visible in World Tree; installation alone does not authorize execution.</p>}<details><summary>Review a source for instruction-only analysis</summary><div className="compatibility-form"><label>Capability type<select aria-label="Review capability type" value={kind} onChange={e=>{setKind(e.target.value);setNetworkApproved(false)}}><option value="skill">Skill</option><option value="plugin">Plugin Skill component</option><option value="mcp">Public documentation MCP</option></select></label>{kind==='mcp'&&<label className="compatibility-network"><span><input type="checkbox" checked={networkApproved} onChange={e=>setNetworkApproved(e.target.checked)}/> Allow discovery at developers.openai.com/mcp</span><small>Connects to list public tool schemas. Uses no account or project files.</small></label>}{kind!=='mcp'&&<><label>Source directory<input aria-label="Review source directory" value={root} onChange={e=>setRoot(e.target.value)}/></label><label>GitHub repository<input aria-label="Review repository" value={repository} onChange={e=>setRepository(e.target.value)} placeholder="https://github.com/owner/repository"/></label><label>Fixed commit SHA<input aria-label="Review commit" value={commit} onChange={e=>setCommit(e.target.value)} maxLength={40}/></label>{kind==='plugin'&&<label>Bundled Skill path<input aria-label="Review Plugin component" value={component} onChange={e=>setComponent(e.target.value)}/></label>}</>}<button disabled={disabled||busy||!isTauri()||(kind==='mcp'?!networkApproved:(!root||!repository||commit.length!==40))} onClick={()=>void review()}>{busy?'Reviewing source…':kind==='mcp'?'Review public MCP connection':'Review source files'}</button></div><p>Skills and templates are read as data. Scripts, Hooks, private accounts and arbitrary shell commands cannot run through this review. Source provenance is supplied metadata bound to local hashes; verify downloaded Git blob hashes before trusting a publisher.</p></details>{error&&<p role="alert" className="mission-error">{error}</p>}</details>
}
