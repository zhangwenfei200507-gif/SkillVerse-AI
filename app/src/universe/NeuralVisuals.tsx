/* eslint-disable react/immutability -- Frame callbacks own mutable GPU attributes and transforms. */
import { useEffect, useLayoutEffect, useMemo, useRef } from 'react'
import { useFrame } from '@react-three/fiber'
import { AdditiveBlending, Color, DynamicDrawUsage, InstancedMesh, Mesh, Object3D, ShaderMaterial, Vector3 } from 'three'
import { noise3D } from './cosmicShaders'

const sphereVertex = `
varying vec3 surface; varying vec3 facing; varying vec3 eye;
void main() {
  surface = position; facing = normalize(normalMatrix * normal);
  vec4 p = modelViewMatrix * vec4(position, 1.);
  eye = -p.xyz; gl_Position = projectionMatrix * p;
}`
const sphereFragment = `
varying vec3 surface; varying vec3 facing; varying vec3 eye;
uniform vec3 tint; uniform float detail; uniform float selected;
${noise3D}
void main() {
  vec3 p=normalize(surface), n=normalize(facing), v=normalize(eye);
  float broad=fbm(p*4.6+vec3(8.6,2.3,1.));
  float rock=fbm(p*18.+broad*3.7);
  float grain=noise3(p*61.);
  float ground=smoothstep(.27,.76,broad*.56+rock*.36+grain*.08);
  float sunlight=smoothstep(-.24,.76,dot(n,normalize(vec3(-.62,.68,.39))));
  float mineral=smoothstep(.36,.78,rock)*(.68+grain*.32);
  float rim=pow(1.-max(dot(n,v),0.),3.8);
  float seam=1.-smoothstep(.012,.045,abs(rock-.53));
  vec3 stone=mix(vec3(.006,.011,.025),tint*(.085+.26*mineral),ground);
  vec3 col=stone*(.36+sunlight*1.8)+tint*seam*ground*(.045+detail*.13);
  col+=tint*rim*(.14+sunlight*.44)+tint*selected*.033;
  gl_FragColor=vec4(col,1.);
  #include <tonemapping_fragment>
  #include <colorspace_fragment>
}`

export function PlanetSurface({ color, core = false, selected = false }: { color: string; core?: boolean; selected?: boolean }) {
  const uniforms = useMemo(() => ({
    tint: { value: new Color(color) },
    detail: { value: Number(core) },
    selected: { value: Number(selected) },
  }), [color, core, selected])
  return <shaderMaterial uniforms={uniforms} vertexShader={sphereVertex} fragmentShader={sphereFragment} />
}

function sphereDirections(count: number) {
  return Array.from({ length: count }, (_, i) => {
    const y = 1 - (i + .5) * 2 / count
    const width = Math.sqrt(1 - y * y)
    return new Vector3(Math.cos(i * 2.39996) * width, y, Math.sin(i * 2.39996) * width)
  })
}

const blue = '#6acbff'
const violet = '#9c8dff'
const rose = '#ffc3eb'

function latticeData(quality: 'low' | 'balanced' | 'high') {
  const count = quality === 'low' ? 45 : 76
  const points = sphereDirections(count)
  const positions: number[] = []
  const colors: number[] = []
  const middle = new Vector3()
  const tint = new Color()
  for (let shell = 0; shell < 2; shell++) {
    const radius = shell ? 1.48 : 1.365
    for (const [i, from] of points.entries()) {
      const nearest = points.map((to, j) => ({ j, distance: from.distanceToSquared(to) }))
        .filter(({ j }) => j !== i).sort((a, b) => a.distance - b.distance).slice(0, shell ? 2 : 3)
      for (const { j } of nearest) {
        if (j < i) continue
        tint.set((i + j) % 11 === 0 ? rose : (i + j) % 4 === 0 ? violet : blue)
        tint.multiplyScalar(shell ? .47 : .92)
        for (let step = 0; step < 8; step++) for (const t of [step / 8, (step + 1) / 8]) {
          middle.copy(from).lerp(points[j], t).normalize().multiplyScalar(radius + .046 * Math.sin(t * Math.PI))
          positions.push(middle.x, middle.y, middle.z)
          colors.push(tint.r, tint.g, tint.b)
        }
      }
    }
  }
  return { positions: new Float32Array(positions), colors: new Float32Array(colors), points }
}

export function NeuralLattice({ quality }: { quality: 'low' | 'balanced' | 'high' }) {
  const { positions, colors, points } = useMemo(() => latticeData(quality), [quality])
  const nodes = useRef<InstancedMesh>(null)
  useLayoutEffect(() => {
    const target = nodes.current
    if (!target) return
    const matrix = new Object3D()
    const tint = new Color()
    points.forEach((position, index) => {
      matrix.position.copy(position).multiplyScalar(index % 4 === 0 ? 1.49 : 1.38)
      matrix.scale.setScalar(index % 9 === 0 ? .041 : .015)
      matrix.updateMatrix()
      target.setMatrixAt(index, matrix.matrix)
      target.setColorAt(index, tint.set(index % 9 === 0 ? rose : blue).multiplyScalar(index % 9 === 0 ? 2 : 1.12))
    })
    target.instanceMatrix.needsUpdate = true
    if (target.instanceColor) target.instanceColor.needsUpdate = true
  }, [points])
  return <group name="neural-lattice">
    <lineSegments>
      <bufferGeometry><bufferAttribute attach="attributes-position" args={[positions, 3]} /><bufferAttribute attach="attributes-color" args={[colors, 3]} /></bufferGeometry>
      <lineBasicMaterial vertexColors transparent opacity={.73} depthWrite={false} blending={AdditiveBlending} toneMapped={false} />
    </lineSegments>
    <instancedMesh key={points.length} ref={nodes} args={[undefined, undefined, points.length]}>
      <icosahedronGeometry args={[1, 0]} /><meshBasicMaterial toneMapped={false} />
    </instancedMesh>
  </group>
}

const ringVertex = `
varying vec2 point; void main(){point=position.xy;gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.);}`
const ringFragment = `
varying vec2 point; uniform vec3 tint; uniform float elapsed;
void main(){
  float radius=length(point); float wave=clamp(elapsed,0.,1.);
  float a=exp(-pow((radius-.29-wave*.70)*36.,2.))*(1.-wave);
  float trail=clamp((elapsed-.16)/.84,0.,1.);
  a+=exp(-pow((radius-.26-trail*.73)*30.,2.))*(1.-trail)*smoothstep(.16,.29,elapsed)*.47;
  gl_FragColor=vec4(tint,a*.7);
}`
export function ClickWave({ radius, color, motion, token }: { radius: number; color: string; motion: number; token: number }) {
  const mesh = useRef<Mesh>(null)
  const material = useRef<ShaderMaterial>(null)
  const started = useRef(-10000)
  const uniforms = useMemo(() => ({ tint: { value: new Color(color).multiplyScalar(1.6) }, elapsed: { value: 1 } }), [color])
  useEffect(() => { if (token) started.current = performance.now() }, [token])
  useFrame(({ camera }) => {
    if (!mesh.current || !material.current) return
    const elapsed = (performance.now() - started.current) / 880
    mesh.current.visible = Boolean(motion && elapsed < 1)
    if (!mesh.current.visible) return
    mesh.current.quaternion.copy(camera.quaternion)
    material.current.uniforms.elapsed.value = elapsed
  })
  return <mesh ref={mesh} visible={false} name="interaction-wave" position={[0, 0, radius * .15]} scale={radius * 1.98}>
    <planeGeometry args={[2, 2]} /><shaderMaterial ref={material} uniforms={uniforms} vertexShader={ringVertex} fragmentShader={ringFragment} transparent depthWrite={false} blending={AdditiveBlending} toneMapped={false} />
  </mesh>
}

export function CoreOrbits({ motion, wide }: { motion: number; wide: boolean }) {
  const satellites = useRef<InstancedMesh>(null)
  const matrix = useMemo(() => new Object3D(), [])
  const time = useRef(0)
  const colors = useMemo(() => [blue, violet, rose].map((color) => new Color(color).multiplyScalar(1.9)), [])
  const orbitCount = wide ? 3 : 2
  useFrame((_, delta) => {
    time.current += Math.min(delta, .045) * motion
    if (!satellites.current) return
    for (let i = 0; i < orbitCount * 3; i++) {
      const row = Math.floor(i / 3)
      const angle = i * 2.094 + time.current * (.46 - row * .13)
      const radius = (wide ? 1.9 : 1.72) + row * .21
      matrix.position.set(Math.cos(angle) * radius, Math.sin(angle) * radius * .43, Math.sin(angle) * .91)
      matrix.position.applyAxisAngle(Vector3Axis, -.28 + row * .32)
      matrix.scale.setScalar(i % 3 === 0 ? .036 : .018)
      matrix.updateMatrix()
      satellites.current.setMatrixAt(i, matrix.matrix)
    }
    satellites.current.instanceMatrix.needsUpdate = true
  })
  useLayoutEffect(() => {
    const target = satellites.current
    if (!target) return
    target.instanceMatrix.setUsage(DynamicDrawUsage)
    for (let i = 0; i < orbitCount * 3; i++) target.setColorAt(i, colors[Math.floor(i / 3)])
    if (target.instanceColor) target.instanceColor.needsUpdate = true
  }, [colors, orbitCount])
  return <group name="neural-orbits">
    {Array.from({ length: orbitCount }, (_, index) => {
      const radius = (wide ? 1.9 : 1.72) + index * .21
      return <group key={index} rotation-z={-.28 + index * .32}><mesh rotation-x={-.84} scale={[1, .68, 1]}><torusGeometry args={[radius, index === 0 ? .012 : .006, 5, 100]} /><meshBasicMaterial color={colors[index]} transparent opacity={index === 0 ? .67 : .43} depthWrite={false} toneMapped={false} /></mesh></group>
    })}
    <instancedMesh key={orbitCount} ref={satellites} args={[undefined, undefined, orbitCount * 3]} frustumCulled={false}>
      <icosahedronGeometry args={[1, 0]} /><meshBasicMaterial toneMapped={false} />
    </instancedMesh>
  </group>
}

const Vector3Axis = new Vector3(0, 0, 1)
