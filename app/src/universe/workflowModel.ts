export type WorkflowNode={id:string;capabilityId:string|null;capabilityType:string;capabilityVersion:string|null;description:string;dependencies:string[];inputSchema:unknown;outputSchema:unknown;adapter:string;risk:string;estimatedCost:string;timeoutMs:number;maxAttempts:number;retryPolicy:string;verificationPolicy:string;params:Record<string,unknown>;condition:{node:string;field:string;equals:unknown}|null}
type ProjectRoutingEvidence = {
  executionProfile?: string
  planning?: {
    mode?: string
    cached?: boolean
    ledger?: {requestId: string; provider: string; model: string; reservedMicrousd: number; settledMicrousd: number | null}
  }
}
export type WorkflowPlan={workflowId:string;revision:number;goal:string;projectRoot:string;complexity:string;scope:string;goalVerifiable:boolean;nodes:WorkflowNode[];routing:{recommendations?:Array<{logicalId:string;entityId:string;name:string;kind:string;reason:string;runtime:string;nextStep:string}>;selectionRule?:string;simplerAlternative?:string;omitted?:string;projectRouting?:ProjectRoutingEvidence}}
export type WorkflowStep={id:string;nodeId:string;attempt:number;status:string;startedAt:number;finishedAt:number|null;output:unknown;inputDigest:string;outputDigest:string|null;errorType:string;sideEffect:string}
export type WorkflowRun={id:string;revision:number;plan?:WorkflowPlan;verificationStatus?:string;reportIntact?:boolean;goalVerified?:boolean;mode:string;status:string;lastError:string;createdAt:number;startedAt:number|null;finishedAt:number|null;workspace:string;scopeDigest:string;scope:unknown;steps:WorkflowStep[];evidence:Array<{id:string;nodeId:string;location:string;sha256:string;intact:boolean;policy:string}>}
export type WorkflowState={ready:boolean;isolated?:boolean;defaultProject?:string;workflows?:Array<{id:string;title:string;revision:number;updatedAt:number}>;plan?:WorkflowPlan;digest?:string;runs?:WorkflowRun[]}
export type WorkflowEvent={sequence:number;nodeId:string|null;executionId:string|null;event:string;at:number;data:unknown}
export type WorkflowSubject={logicalId:string;entityId:string;name:string;kind:string;version:string;runtime:string}
export const isActive=(status:string)=>['QUEUED','RUNNING'].includes(status)
export function latestSteps(run?:WorkflowRun){const result=new Map<string,WorkflowStep>();for(const step of run?.steps??[]){const previous=result.get(step.nodeId);if(!previous||previous.attempt<=step.attempt)result.set(step.nodeId,step)}return result}
export function initialPositions(plan:WorkflowPlan){const levels=new Map<string,number>();const pending=[...plan.nodes];for(let pass=0;pass<33&&pending.length;pass++){for(let i=pending.length-1;i>=0;i--){const n=pending[i];if(n.dependencies.every(d=>levels.has(d))){levels.set(n.id,n.dependencies.length?Math.max(...n.dependencies.map(d=>levels.get(d)!))+1:0);pending.splice(i,1)}}}const rows=new Map<number,number>();const positions:Record<string,{x:number;y:number}>={};for(const n of plan.nodes){const level=levels.get(n.id)??0;const row=rows.get(level)??0;rows.set(level,row+1);positions[n.id]={x:300+level*340,y:row*178+50}}return positions}
export function removeWorkflowNode(plan:WorkflowPlan,id:string):WorkflowPlan{return {...plan,goalVerifiable:false,nodes:plan.nodes.filter(n=>n.id!==id).map(n=>({...n,dependencies:n.dependencies.filter(d=>d!==id),condition:n.condition?.node===id?null:n.condition}))}}
export function scopeLabel(run?:WorkflowRun){if(!run)return 'Plan only · nothing has run';if(run.mode==='DRY_RUN')return 'Dry run · no adapter effects or execution evidence';return 'Approved execution scope · runtime evidence and mastery stay independent'}
