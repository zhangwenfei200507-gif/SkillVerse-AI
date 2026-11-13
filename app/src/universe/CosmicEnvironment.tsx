/* eslint-disable react/immutability -- The frame loop owns GPU uniforms and scene transforms. */
import { useEffect, useLayoutEffect, useMemo, useRef } from 'react'
import { useFrame, useThree } from '@react-three/fiber'
import { Sparkles, Stars } from '@react-three/drei'
import { AdditiveBlending, BackSide, Color, IcosahedronGeometry, InstancedMesh, Matrix4, Mesh, Object3D, ShaderMaterial } from 'three'
import { cosmicBudget, cosmicThemes, distantGalaxyData, luminousStarData, type CosmicQuality, type UniverseTheme } from './cosmicModel'
import { luminousFragment, luminousVertex, skyFragment, skyVertex } from './cosmicShaders'
import NebulaVolume from './NebulaVolume'

interface Props { page: UniverseTheme; quality: CosmicQuality; density: number; motion: number; nebula: boolean; stars: boolean; particles: boolean }
const rocks = [
  [-8.8, 3.25, -1.3, .44], [-6.2, 4.4, -3, .30], [-7.4, -4.55, .5, .51], [-4.9, -3.7, -2, .23],
  [-3.9, 3.75, -4, .15], [.2, 5.5, -5, .34], [-10.1, .7, -7, .35], [3.4, -5.4, -4, .33],
  [-1.8, -4.8, -2, .12], [-2.5, 3.25, -4, .11], [5.4, 2.6, -5, .17], [5.8, -4.25, -8, .21],
]
function DriftingDebris({ page, quality, motion }: { page: UniverseTheme; quality: CosmicQuality; motion: number }) {
  const group = useRef<InstancedMesh>(null)
  const count = quality === 'low' ? 6 : rocks.length
  const geometry = useMemo(() => {
    const result = new IcosahedronGeometry(1, 2)
    const vertices = result.getAttribute('position')
    for (let i = 0; i < vertices.count; i++) {
      const x = vertices.getX(i), y = vertices.getY(i), z = vertices.getZ(i)
      const ridge = 1 + .16 * Math.sin(x * 7.9 + y * 9.2) * Math.sin(z * 9.1 + x * 4.7)
      vertices.setXYZ(i, x * ridge, y * ridge, z * ridge)
    }
    result.computeVertexNormals()
    return result
  }, [])
  useEffect(() => () => geometry.dispose(), [geometry])
  useLayoutEffect(() => {
    const target = group.current
    if (!target) return
    const object = new Object3D()
    const tint = new Color()
    for (let i = 0; i < count; i++) {
      const [x, y, z, scale] = rocks[i]
      object.position.set(x, y, z)
      object.rotation.set(i * 1.23, i * .72, i * 2.39)
      object.scale.set(scale, scale * .82, scale * 1.19)
      object.updateMatrix()
      target.setMatrixAt(i, object.matrix)
      target.setColorAt(i, tint.set(i % 3 ? '#35456a' : '#455879'))
    }
    target.instanceMatrix.needsUpdate = true
    if (target.instanceColor) target.instanceColor.needsUpdate = true
    target.computeBoundingSphere()
  }, [count])
  useFrame((_, delta) => {
    if (group.current && motion) group.current.rotation.z += Math.min(delta, .045) * motion * .006
  })
  return <instancedMesh key={count} ref={group} name="cosmic-drifting-debris" args={[undefined, undefined, count]} position={page === 'Learning' ? [-1, .5, -2] : [0, 0, -2.1]}>
    <primitive object={geometry} attach="geometry" />
    <meshStandardMaterial roughness={.94} metalness={.18} flatShading />
  </instancedMesh>
}
export default function CosmicEnvironment({page,quality,density,motion,nebula,stars,particles}:Props) {
  const {camera,size,gl}=useThree()
  const sky=useRef<Mesh>(null)
  const volume=useRef<Mesh>(null)
  const luminous=useRef<ShaderMaterial>(null)
  const budget=cosmicBudget(quality,density,size.width<760)
  const data=useMemo(()=>luminousStarData(budget.luminous),[budget.luminous])
  const galaxy=useMemo(()=>distantGalaxyData(quality==='low'?700:2200),[quality])
  const targets=useMemo(()=>{
    const theme=cosmicThemes[page]
    return {deep:new Color(theme.deep),lift:new Color(theme.lift),gas:new Color(theme.gas),accent:new Color(theme.accent),strength:theme.strength}
  },[page])
  const uniforms=useMemo(()=>({
    time:{value:0},strength:{value:cosmicThemes['World Tree'].strength},
    deepColor:{value:new Color(cosmicThemes['World Tree'].deep)},liftColor:{value:new Color(cosmicThemes['World Tree'].lift)},
    gasColor:{value:new Color(cosmicThemes['World Tree'].gas)},accentColor:{value:new Color(cosmicThemes['World Tree'].accent)},
    worldToLocal:{value:new Matrix4()},
  }),[])
  const starUniforms=useMemo(()=>({time:{value:0},pixelRatio:{value:1},opacity:{value:1}}),[])
  const galaxyUniforms=useMemo(()=>({time:{value:0},pixelRatio:{value:1},opacity:{value:.55}}),[])
  useFrame((_,delta)=>{
    sky.current?.position.copy(camera.position)
    const blend=motion?1-Math.exp(-Math.min(delta,.08)*1.8):1
    uniforms.deepColor.value.lerp(targets.deep,blend);uniforms.liftColor.value.lerp(targets.lift,blend)
    uniforms.gasColor.value.lerp(targets.gas,blend);uniforms.accentColor.value.lerp(targets.accent,blend)
    uniforms.strength.value+=(targets.strength-uniforms.strength.value)*blend
    uniforms.time.value+=Math.min(delta,.08)*motion
    if(volume.current){volume.current.updateWorldMatrix(true,false);uniforms.worldToLocal.value.copy(volume.current.matrixWorld).invert()}
    if(luminous.current){starUniforms.time.value=uniforms.time.value;starUniforms.pixelRatio.value=gl.getPixelRatio()}
    galaxyUniforms.time.value=uniforms.time.value;galaxyUniforms.pixelRatio.value=gl.getPixelRatio()
  },-4)
  return <group name="cosmic-environment">
    <mesh ref={sky} name="cosmic-deep-space" renderOrder={-100} frustumCulled={false}>
      <sphereGeometry args={[170,32,24]}/><shaderMaterial key={quality === 'low' ? 'fast' : 'full'} side={BackSide} depthWrite={false} depthTest={false} uniforms={uniforms} defines={{ FAST_SKY: quality === 'low' ? 1 : 0 }} vertexShader={skyVertex} fragmentShader={skyFragment}/>
    </mesh>
    <NebulaVolume volume={volume} uniforms={uniforms} quality={quality} steps={budget.steps} enabled={nebula}/>
    {stars&&<>
    <group name="drei-distant-stars"><Stars radius={48} depth={65} count={budget.distant} factor={3.1} saturation={.12} fade speed={motion*.14}/></group>
    <points name="cosmic-distant-galaxy" position={[-23,21,-75]} rotation={[.62,-.2,.2]}>
      <bufferGeometry>
        <bufferAttribute attach="attributes-position" args={[galaxy.position,3]}/><bufferAttribute attach="attributes-color" args={[galaxy.color,3]}/>
        <bufferAttribute attach="attributes-starSize" args={[galaxy.size,1]}/><bufferAttribute attach="attributes-phase" args={[galaxy.phase,1]}/><bufferAttribute attach="attributes-twinkleSpeed" args={[galaxy.speed,1]}/>
      </bufferGeometry>
      <shaderMaterial vertexColors transparent depthWrite={false} blending={AdditiveBlending} toneMapped={false} uniforms={galaxyUniforms} vertexShader={luminousVertex} fragmentShader={luminousFragment}/>
    </points>
    <group name="drei-middle-stars"><Stars radius={18} depth={40} count={budget.middle} factor={2.2} saturation={.18} fade speed={motion*.22}/></group>
    <points name="cosmic-luminous-stars" frustumCulled={false}>
      <bufferGeometry>
        <bufferAttribute attach="attributes-position" args={[data.position,3]}/><bufferAttribute attach="attributes-color" args={[data.color,3]}/>
        <bufferAttribute attach="attributes-starSize" args={[data.size,1]}/><bufferAttribute attach="attributes-phase" args={[data.phase,1]}/><bufferAttribute attach="attributes-twinkleSpeed" args={[data.speed,1]}/>
      </bufferGeometry>
      <shaderMaterial ref={luminous} vertexColors transparent depthWrite={false} blending={AdditiveBlending} toneMapped={false} uniforms={starUniforms} vertexShader={luminousVertex} fragmentShader={luminousFragment}/>
    </points>
    </>}
    {particles&&<>
    <Sparkles name="drei-spatial-sparkles" position={[-3,2,-5]} scale={[26,20,18]} count={Math.round(60*density)} size={1.25} speed={motion*.08} opacity={.18} color="#aacfff" noise={.2}/>
    <Sparkles name="cosmic-foreground-particles" position={[0,2,5]} scale={[36,26,12]} count={budget.foreground} size={1.05} speed={motion*.055} opacity={.12} color="#c0d9fa" noise={.12}/>
    </>}
    {page !== 'Settings' && page !== 'Mission Control' && <DriftingDebris page={page} quality={quality} motion={motion} />}
  </group>
}
