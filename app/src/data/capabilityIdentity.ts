/** Provenance is supplied by the native resolver; a display name never creates a link. */
export interface PhysicalCapabilityObservation {
 entityId:string;physicalId:string;logicalId:string;kind:string;machineName:string;originKey:string;authority:Record<string,unknown>;componentKey:string;scopeKey:string;location:string;manifestPath?:string;packageVersion?:string;cacheVersion?:string;manifestHash:string;present:boolean;status:string;confidence:string;evidence:Record<string,unknown>
}
export interface LocalIdentityScan {
 context:{userHome:string;codexHome:string;projectRoot?:string;userKey:string;projectKey?:string};observations:PhysicalCapabilityObservation[];history:PhysicalCapabilityObservation[];warnings:string[];duplicateManifests?:Array<Record<string,unknown>>
}
