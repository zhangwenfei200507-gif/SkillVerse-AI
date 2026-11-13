/* eslint-disable react/immutability -- A bounded offscreen pass owns its renderer state. */
import { useMemo, type RefObject } from 'react'
import { createPortal, useFrame, useThree } from '@react-three/fiber'
import { useFBO } from '@react-three/drei'
import { BackSide, Color, HalfFloatType, Mesh, NoBlending, Scene, type IUniform } from 'three'
import { nebulaFragment, nebulaVertex } from './cosmicShaders'
import type { CosmicQuality } from './cosmicModel'

interface Props {
  volume: RefObject<Mesh | null>; uniforms: Record<string,IUniform>
  quality: CosmicQuality; steps: number; enabled: boolean
}
export default function NebulaVolume({volume,uniforms,quality,steps,enabled}:Props) {
  const {gl,camera,size}=useThree()
  const dpr=useThree(state=>state.viewport.dpr)
  const ratio=quality==='low'?.20:quality==='high'?.67:.5
  const width=Math.max(1,Math.min(1536,Math.round(size.width*dpr*ratio)))
  const height=Math.max(1,Math.min(1024,Math.round(size.height*dpr*ratio)))
  const target=useFBO(width,height,{type:HalfFloatType,depthBuffer:false,stencilBuffer:false})
  const scene=useMemo(()=>new Scene(),[])
  const previousColor=useMemo(()=>new Color(),[])
  const outputUniforms=useMemo(()=>({gasBuffer:{value:target.texture}}),[target.texture])
  useFrame(()=>{
    if(!enabled)return
    const previousTarget=gl.getRenderTarget()
    const previousAlpha=gl.getClearAlpha()
    gl.getClearColor(previousColor)
    try {
      gl.setRenderTarget(target);gl.setClearColor(0x000000,0);gl.clear()
      gl.render(scene,camera)
    } finally {
      gl.setRenderTarget(previousTarget);gl.setClearColor(previousColor,previousAlpha)
    }
    // The evidence refers to the real raymarched mesh, not its compositing quad.
    if(volume.current)volume.current.userData.bufferSize=[width,height]
  })
  return <>
    {createPortal(<mesh ref={volume} name="cosmic-nebula-density" position={[-2,9,-45]} scale={[105,75,48]}>
      <boxGeometry/><shaderMaterial key={steps} side={BackSide} blending={NoBlending} depthWrite={false} uniforms={uniforms} defines={{VOLUME_STEPS:steps}} vertexShader={nebulaVertex} fragmentShader={nebulaFragment}/>
    </mesh>,scene)}
    <mesh name="cosmic-nebula-volume" renderOrder={-80} visible={enabled} frustumCulled={false} userData={{densityMesh:volume}}>
      <planeGeometry args={[2,2]}/>
      <shaderMaterial transparent depthWrite={false} uniforms={outputUniforms}
        vertexShader="varying vec2 uv0;void main(){uv0=uv;gl_Position=vec4(position.xy,.99999,1.);}"
        fragmentShader={`
          varying vec2 uv0;uniform sampler2D gasBuffer;
          void main(){
            gl_FragColor=texture2D(gasBuffer,uv0);
            #include <tonemapping_fragment>
            #include <colorspace_fragment>
          }
        `}/>
    </mesh>
  </>
}
