import { Mesh, Points, ShaderMaterial, Vector3 } from 'three'

export interface CosmicEvidence {
  layers: number; nebulaTechnique: string; steps: number; luminousStars: number
  foreground: number; depthSpan: number; bufferSize: number[]; probes: Array<{ world: number[]; screen: number[] }>
}
export function inspectCosmicEnvironment(scene: import('three').Scene, camera: import('three').Camera, size: {width:number;height:number}): CosmicEvidence | undefined {
  const field = scene.getObjectByName('cosmic-nebula-volume') as Mesh | undefined
  const stars = scene.getObjectByName('cosmic-luminous-stars') as Points | undefined
  if (!field || !stars) return undefined
  const positions = stars.geometry.getAttribute('position')
  const depth: number[] = []
  for(let i=0;i<positions.count;i++) depth.push(positions.getZ(i))
  const point=new Vector3()
  if(stars.userData.probeGeometry!==stars.geometry){
    const visible:number[]=[]
    for(let i=0;i<positions.count;i++){
      point.fromBufferAttribute(positions,i).applyMatrix4(stars.matrixWorld).project(camera)
      if(Math.abs(point.x)<.65&&Math.abs(point.y)<.65&&point.z>-1&&point.z<1)visible.push(i)
    }
    visible.sort((a,b)=>positions.getZ(b)-positions.getZ(a))
    stars.userData.probeIndices=visible.length>=2?[visible[0],visible[visible.length-1],visible[Math.floor(visible.length/3)],visible[Math.floor(visible.length*2/3)]]:Array.from({length:Math.min(4,positions.count)},(_,i)=>i)
    stars.userData.probeGeometry=stars.geometry
  }
  const probes=(stars.userData.probeIndices as number[]).map((i)=>{
    point.fromBufferAttribute(positions,i).applyMatrix4(stars.matrixWorld)
    const world=point.toArray();point.project(camera)
    return {world,screen:[(point.x+1)*size.width/2,(1-point.y)*size.height/2]}
  })
  const densityMesh=field.userData.densityMesh.current as Mesh
  return {layers:6,nebulaTechnique:field.visible?'bounded-3d-density-raymarch':'disabled',steps:Number((densityMesh.material as ShaderMaterial).defines.VOLUME_STEPS),bufferSize:densityMesh.userData.bufferSize??[],luminousStars:positions.count,foreground:(scene.getObjectByName('cosmic-foreground-particles') as Points)?.geometry.getAttribute('position').count ?? 0,depthSpan:Math.max(...depth)-Math.min(...depth),probes}
}
