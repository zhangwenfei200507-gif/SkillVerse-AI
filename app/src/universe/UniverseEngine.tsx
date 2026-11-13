/* eslint-disable react/immutability -- R3F's useFrame owns mutable Three.js camera, scene, and geometry objects outside React state. */
import { Component, Suspense, useEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from 'react'
import { Canvas, useFrame, useThree, type ThreeEvent } from '@react-three/fiber'
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { Environment, Html, Lightformer, Line, OrbitControls, Sparkles } from '@react-three/drei'
import { Bloom, EffectComposer, ToneMapping } from '@react-three/postprocessing'
import { ToneMappingMode } from 'postprocessing'
import { AudioLines, BookOpen, Box, CircleHelp, Clapperboard, Cloud, Code2, FileText, GitBranch, Globe2, GraduationCap, Layers3, Orbit, Palette, Presentation, Server, ShieldCheck, Sparkles as SparkleIcon, Table2, Workflow, Zap } from 'lucide-react'
import { AdditiveBlending, Color, CubicBezierCurve3, DataTexture, Group, LinearFilter, Mesh, MOUSE, Plane, Points, RGBAFormat, Sprite, TOUCH, Vector3 } from 'three'
import type { Line2 } from 'three-stdlib'
import type { OrbitControls as OrbitControlsImpl } from 'three-stdlib'
import type { ScenePage, SpatialNode, Positions, Point3 } from './model'
import { nodeRadius, spatialConnections, type SpatialConnection } from './spatialLayout'
import UniverseFallback from './UniverseFallback'
import CosmicEnvironment from './CosmicEnvironment'
import { inspectCosmicEnvironment, type CosmicEvidence } from './cosmicEvidence'
import { ClickWave, CoreOrbits, NeuralLattice, PlanetSurface } from './NeuralVisuals'

export interface UniverseSettings {
  quality: 'low' | 'balanced' | 'high'
  density: number
  motion: number
  reducedMotion: boolean
  restoreScan: boolean
  autoScan: boolean
  stars: boolean
  particles: boolean
  rendering: 'auto' | '3d' | '2d'
  nebula: boolean
  bloom: number
}
export interface RenderMetrics { software?: boolean; fps: number; p95: number; calls: number; triangles: number; geometries: number; textures: number; stars: number; sparkles: number; renderer: string; cosmic?: CosmicEvidence; bloom: boolean }
interface Props {
  page: ScenePage
  nodes: SpatialNode[]
  selectedId: string
  positions: Positions
  settings: UniverseSettings
  focus: boolean
  background?: boolean
  zoom: number
  fitToken: number
  pulseToken: number
  onSelect: (node: SpatialNode) => void
  onMove: (id: string, position: Point3, commit?: boolean) => Point3
  onMetrics: (metrics: RenderMetrics) => void
}
const icons: Record<string, typeof Orbit> = {
  Engineering: Code2, Knowledge: BookOpen, Automation: Zap, Frontend: Palette, Unclassified: CircleHelp,
  'Pending classification': CircleHelp, Skills: FileText, 'MCP Servers': Workflow, Plugins: Box,
  Explore: Globe2, Configure: Workflow, Practice: FileText, Verify: ShieldCheck, Master: GraduationCap,
  'Presentations / PPT': Presentation, 'Video & animation': Clapperboard, 'Frontend interfaces': Palette, 'Backend & APIs': Server,
  'Documents & PDF': FileText, Spreadsheets: Table2, 'Browser automation': Workflow, 'Cloud & deployment': Cloud,
  'Code & developer tools': Code2, 'Research & knowledge': BookOpen, 'Images & graphics': Palette, 'Audio & speech': AudioLines,
}
function NodeIcon({ node }: { node: SpatialNode }) {
  const capabilityIcon = node.capability && (node.capability.kind === 'mcp' ? GitBranch : node.capability.kind === 'plugin' ? Box : BookOpen)
  const Icon = capabilityIcon || icons[node.name] || (node.kind === 'cluster' ? Layers3 : SparkleIcon)
  return <Icon size={node.kind === 'domain' ? 24 : 21} strokeWidth={1.45} />
}
const rimVertex = 'varying vec3 n;varying vec3 v;void main(){vec4 p=modelViewMatrix*vec4(position,1.);n=normalize(normalMatrix*normal);v=normalize(-p.xyz);gl_Position=projectionMatrix*p;}'
const rimFragment = 'varying vec3 n;varying vec3 v;uniform vec3 tint;void main(){float f=pow(1.-max(dot(normalize(n),normalize(v)),0.),3.2);gl_FragColor=vec4(tint,f*.65);}'
function Atmosphere({ radius, color }: { radius: number; color: string }) {
  const uniforms = useMemo(() => ({ tint: { value: new Color(color) } }), [color])
  return <mesh><sphereGeometry args={[radius, 48, 32]} /><shaderMaterial transparent depthWrite={false} blending={AdditiveBlending} uniforms={uniforms} vertexShader={rimVertex} fragmentShader={rimFragment} /></mesh>
}
function glowTexture() {
  const data = new Uint8Array(64 * 64 * 4)
  for (let y = 0; y < 64; y++) for (let x = 0; x < 64; x++) {
    const distance = Math.hypot((x - 31.5) / 31.5, (y - 31.5) / 31.5)
    const index = (y * 64 + x) * 4
    data[index] = data[index + 1] = data[index + 2] = 255
    data[index + 3] = Math.round(Math.pow(Math.max(0, 1 - distance), 3.4) * 255)
  }
  const texture = new DataTexture(data, 64, 64, RGBAFormat)
  texture.magFilter = LinearFilter
  texture.minFilter = LinearFilter
  texture.needsUpdate = true
  return texture
}
function NodeGlow({ radius, color, selected, motion, pulseToken, texture }: { radius: number; color: string; selected: boolean; motion: number; pulseToken: number; texture: DataTexture }) {
  const sprite = useRef<Sprite>(null)
  const started = useRef(-10000)
  useEffect(() => { if (selected && pulseToken) started.current = performance.now() }, [selected, pulseToken])
  useFrame(() => {
    if (!sprite.current || !selected) return
    const seconds = (performance.now() - started.current) / 1000
    const pulse = motion ? Math.exp(-Math.pow((seconds - .1) / .1, 2)) + .65 * Math.exp(-Math.pow((seconds - .48) / .15, 2)) : 0
    sprite.current.material.opacity = .6 + pulse * .27
    sprite.current.scale.setScalar(radius * (5.8 + pulse * 1.3))
  })
  return <sprite ref={sprite} scale={[radius * 5.8, radius * 5.8, 1]} position={[0, 0, -.025]}><spriteMaterial map={texture} color={color} transparent opacity={selected ? .6 : .37} depthWrite={false} blending={AdditiveBlending} toneMapped={false} /></sprite>
}
function NeuralCore({ node, page, quality, texture, motion, onSelect, selected, pulseToken, caption }: { node: SpatialNode; page: ScenePage; quality: UniverseSettings['quality']; texture: DataTexture; motion: number; onSelect: Props['onSelect']; selected: boolean; pulseToken: number; caption: RefObject<HTMLDivElement | null> }) {
  const root = useRef<Group>(null)
  const [startPosition] = useState(node.position)
  const targetPosition = useMemo(() => new Vector3(...node.position), [node.position])
  const body = useRef<Group>(null)
  const lens = useRef<Group>(null)
  const captionPoint = useMemo(() => new Vector3(), [])
  const wide = page === 'World Tree' || page === 'Discover'
  const color = page === 'Discover' ? '#9f91ff' : page === 'Installed' ? '#6ad7ef' : '#64b4ff'
  useFrame(({ pointer, camera, size }, delta) => {
    if (body.current) body.current.rotation.y += Math.min(delta, .04) * .052 * motion
    if (lens.current) lens.current.rotation.set(.32 + pointer.y * .17 * motion, .46 + pointer.x * .28 * motion, .25)
    if (!root.current) return
    root.current.position.lerp(targetPosition, motion ? Math.min(1, delta * 5) : 1)
    if (caption.current) {
      camera.updateMatrixWorld()
      root.current.localToWorld(captionPoint.set(0, -1.11, 1.47)).project(camera)
      caption.current.style.transform = `translate3d(${(captionPoint.x + 1) * size.width / 2}px, ${(1 - captionPoint.y) * size.height / 2}px, 0)`
    }
  })
  return <group ref={root} position={startPosition} scale={page === 'World Tree' ? 1.45 : page === 'Settings' ? 1.66 : 1.28}>
    <NodeGlow texture={texture} radius={1.53} color={color} selected={selected} motion={motion} pulseToken={pulseToken} />
    <group ref={body}>
      <mesh onClick={(event) => { event.stopPropagation(); onSelect(node) }}><sphereGeometry args={[1.34, 56, 40]} /><PlanetSurface core color={color} selected={selected} /></mesh>
      <NeuralLattice quality={quality} />
      <Atmosphere radius={1.39} color={color} />
      {wide && <Sparkles count={quality === 'low' ? 42 : 82} scale={[3.85, 3.85, 3.85]} size={2.5} speed={motion * .16} color="#bdceff" opacity={.49} noise={.35} />}
    </group>
    <group ref={lens} position={[0, .08, 1.45]}><mesh><octahedronGeometry args={[.32, 0]} /><meshBasicMaterial color="#567fff" transparent opacity={.43} depthWrite={false} /></mesh><mesh><octahedronGeometry args={[.35, 0]} /><meshBasicMaterial color="#91c8ff" wireframe toneMapped={false} /></mesh><NodeGlow texture={texture} radius={.38} color="#b9a4ff" selected={selected} motion={motion} pulseToken={pulseToken} /></group>
    <CoreOrbits motion={motion} wide={wide} />
    {selected && <ClickWave radius={1.46} color={color} motion={motion} token={pulseToken} />}
  </group>
}
function SpatialOrb({ node, selected, motion, targetPosition, focus, onSelect, onMove, onDrag, columnar, movable, pulseToken, texture, low, orbLabels }: {
  node: SpatialNode; selected: boolean; motion: number; targetPosition: Point3; focus: boolean
  onSelect: Props['onSelect']; onMove: Props['onMove']; onDrag: (value: boolean) => void
  columnar: boolean; movable: boolean; pulseToken: number; texture: DataTexture; low: boolean; orbLabels: RefObject<HTMLElement>
}) {
  const root = useRef<Group>(null)
  const dragging = useRef(false)
  const moved = useRef(false)
  const dragOrigin = useRef(new Vector3())
  const current = useRef(new Vector3(...targetPosition))
  const destination = useMemo(() => new Vector3(...targetPosition), [targetPosition])
  const dragPlane = useMemo(() => new Plane(new Vector3(0, 0, 1), -targetPosition[2]), [targetPosition])
  const hit = useMemo(() => new Vector3(), [])
  const radius = nodeRadius(node, columnar)
  const leftColumn = columnar && targetPosition[0] < 0
  useFrame((_, delta) => {
    if (root.current && !dragging.current) {
      current.current.lerp(destination, motion ? Math.min(1, delta * 6) : 1)
      root.current.position.copy(current.current)
    }
  })
  const down = (event: ThreeEvent<PointerEvent>) => {
    if (!movable || event.button !== 0) return
    event.stopPropagation()
    dragging.current = true; moved.current = false; dragOrigin.current.copy(current.current); onDrag(true)
    const target = event.target as HTMLElement
    target.setPointerCapture?.(event.pointerId)
  }
  const move = (event: ThreeEvent<PointerEvent>) => {
    if (!dragging.current || !root.current) return
    event.stopPropagation()
    if (event.ray.intersectPlane(dragPlane, hit)) {
      if (columnar) hit.set(targetPosition[0], Math.max(-12, Math.min(12, hit.y)), targetPosition[2])
      if (!moved.current && hit.distanceTo(dragOrigin.current) <= .04) return
      moved.current = true
      current.current.set(...onMove(node.id, hit.toArray() as Point3))
      root.current.position.copy(current.current)
    }
  }
  const up = (event: ThreeEvent<PointerEvent>) => {
    event.stopPropagation()
    if (!dragging.current) return
    dragging.current = false; onDrag(false)
    const target = event.target as HTMLElement
    target.releasePointerCapture?.(event.pointerId)
    if (moved.current) onMove(node.id, current.current.toArray() as Point3, true)
  }
  return <group ref={root} position={targetPosition}>
    <group>
      {(!low || selected) && <NodeGlow texture={texture} radius={radius} color={node.color} selected={selected} motion={motion} pulseToken={pulseToken} />}
      {node.kind === 'stage' && <group position={[0, -radius - .12, 0]}>
        <mesh position={[0, -.19, 0]} rotation={[.09, .66, -.1]} scale={[.95, .38, .7]}><dodecahedronGeometry args={[1, 1]} /><meshStandardMaterial color="#273553" roughness={.95} flatShading /></mesh>
        <mesh rotation={[Math.PI / 2, 0, 0]} position={[0, .06, 0]}><torusGeometry args={[radius * 1.23, .014, 6, 64]} /><meshBasicMaterial color={node.color} transparent opacity={.76} /></mesh>
      </group>}
      <mesh onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={up} onClick={(event) => { event.stopPropagation(); if (!moved.current) onSelect(node) }}>
        <sphereGeometry args={[radius, low ? 16 : node.kind === 'stage' ? 40 : 24, low ? 12 : 18]} />
        {low && !selected
          ? <meshStandardMaterial color={node.color} roughness={.72} emissive={node.color} emissiveIntensity={.065} />
          : <PlanetSurface color={node.color} selected={selected} />}
      </mesh>
      {(!low || selected) && <Atmosphere radius={radius * 1.065} color={node.color} />}
      {(node.kind === 'stage' || node.capability?.kind === 'mcp' || selected) && <mesh rotation={[Math.PI / 2 + .25, 0, -.2]} position={[0, -radius * .5, 0]}>
        <torusGeometry args={[radius * 1.22, .012, 6, 96]} /><meshBasicMaterial color={node.color} transparent opacity={selected ? .85 : .4} />
      </mesh>}
      {selected && <ClickWave radius={radius} color={node.color} motion={motion} token={pulseToken} />}
      <Html portal={orbLabels} wrapperClass="universe-html-anchor" center distanceFactor={columnar && !node.id.startsWith('function:') ? 17 : undefined} position={[0, 0, radius + .04]} zIndexRange={[15, 0]}><button className={'spatial-orb-icon ' + (columnar ? 'column-orb-icon ' : '') + (selected ? 'selected' : '')} style={{ color: node.color }} onClick={() => onSelect(node)} aria-label={'Select ' + node.name}><NodeIcon node={node} /></button></Html>
    </group>
    <Html portal={orbLabels} wrapperClass="universe-html-anchor" distanceFactor={columnar && !node.id.startsWith('function:') ? 17 : undefined} position={columnar ? [(leftColumn ? -1 : 1) * (radius + .16), 0, .1] : [0, 0, radius + .04]} zIndexRange={[15, 0]}><button className={'spatial-label kind-' + node.kind + (columnar ? ' column-label ' : ' orbital-label ') + (leftColumn ? 'left-column-label ' : '') + (selected ? 'selected' : '') + (focus && node.kind === 'capability' ? ' focus-secondary' : '')} onClick={() => onSelect(node)} aria-pressed={selected} title={node.name} data-node-id={node.id} data-origin={node.capability?.origin} data-state={node.capability?.installation}>
      <strong>{node.name}</strong>{!focus && <small><i style={{ background: node.capability?.installation === 'installed' ? '#78e5ba' : node.color }} />{node.subtitle}</small>}
    </button></Html>
  </group>
}
function nerveCurve({ from, to }: SpatialConnection) {
  const horizontal = (to[0] - from[0]) * .53
  return new CubicBezierCurve3(new Vector3(...from), new Vector3(from[0] + horizontal, from[1], from[2] - .18),
    new Vector3(to[0] - horizontal, to[1], to[2] - .12), new Vector3(...to))
}
function NeuralConnection({ edge, motion, token, transient }: { edge: SpatialConnection; motion: number; token: number; transient: boolean }) {
  const particle = useRef<Mesh>(null)
  const beam = useRef<Line2>(null)
  const started = useRef(-10000)
  const curve = useMemo(() => nerveCurve(edge), [edge])
  const points = useMemo(() => curve.getPoints(36), [curve])
  useEffect(() => { if (token) started.current = performance.now() }, [token])
  useFrame(() => {
    const age = (performance.now() - started.current) / 900
    const progress = motion ? Math.min(1, Math.max(0, age * 1.7)) : 1
    if (beam.current) {
      beam.current.geometry.instanceCount = Math.max(1, Math.ceil(progress * 36))
      beam.current.material.opacity = transient ? .66 * Math.max(0, 1 - age) : .78
      beam.current.visible = !transient || Boolean(motion && age < 1)
    }
    if (!particle.current) return
    particle.current.visible = Boolean(motion && age < 1)
    if (particle.current.visible) curve.getPoint(Math.min(1, age * 1.13), particle.current.position)
  })
  return <group>
    <Line ref={beam} points={points} color={edge.color} lineWidth={2} transparent opacity={.78} depthWrite={false} />
    <mesh ref={particle} visible={false}><sphereGeometry args={[.045, 10, 8]} /><meshBasicMaterial color={[1.8, 2.2, 3]} toneMapped={false} /></mesh>
  </group>
}
function NeuralConnections({ edges, active, selectedId, motion, token }: { edges: SpatialConnection[]; active: Set<string>; selectedId: string; motion: number; token: number }) {
  const { positions, colors } = useMemo(() => {
    const segments = 18
    const positions = new Float32Array(edges.length * segments * 6)
    const colors = new Float32Array(positions.length)
    const point = new Vector3()
    const tint = new Color()
    let offset = 0
    for (const edge of edges) {
      const curve = nerveCurve(edge)
      tint.set(edge.color).multiplyScalar(.86)
      for (let step = 0; step < segments; step++) {
        for (const t of [step / segments, (step + 1) / segments]) {
          curve.getPoint(t, point).toArray(positions, offset)
          const light = .48 + .52 * Math.abs(t * 2 - 1)
          colors.set([tint.r * light, tint.g * light, tint.b * light], offset)
          offset += 3
        }
      }
    }
    return { positions, colors }
  }, [edges])
  return <group name="neural-branches">
    <lineSegments>
      <bufferGeometry><bufferAttribute attach="attributes-position" args={[positions, 3]} /><bufferAttribute attach="attributes-color" args={[colors, 3]} /></bufferGeometry>
      <lineBasicMaterial vertexColors transparent opacity={.64} depthWrite={false} blending={AdditiveBlending} />
    </lineSegments>
    {edges.filter((edge) => active.has(edge.targetId)).map((edge) => <NeuralConnection key={edge.id} edge={edge} motion={motion} token={token} transient={false} />)}
    {Boolean(token && motion && selectedId !== 'codex-root') && edges.filter((edge) => edge.id.startsWith(selectedId + '/')).slice(0, 12)
      .map((edge) => <NeuralConnection key={edge.id} edge={edge} motion={motion} token={token} transient />)}
  </group>
}
const orbitSizes = { 'World Tree': 16.5, Discover: 15.5, Installed: 15.5, Learning: 14.8, Settings: 13 }
function ContextEvents({ onLoss }: { onLoss: (value: boolean) => void }) {
  const { gl, invalidate } = useThree()
  useEffect(() => {
    const canvas = gl.domElement
    const lost = (event: Event) => { event.preventDefault(); onLoss(true) }
    const restored = () => { onLoss(false); invalidate() }
    canvas.addEventListener('webglcontextlost', lost)
    canvas.addEventListener('webglcontextrestored', restored)
    return () => {
      canvas.removeEventListener('webglcontextlost', lost)
      canvas.removeEventListener('webglcontextrestored', restored)
    }
  }, [gl, invalidate, onLoss])
  return null
}
function Scene({ page, nodes, selectedId, positions, settings, focus, background, zoom, fitToken, pulseToken, onSelect, onMove, onMetrics, caption, orbLabels }: Props & { caption: RefObject<HTMLDivElement | null>; orbLabels: RefObject<HTMLElement> }) {
  const controls = useRef<OrbitControlsImpl>(null)
  const [dragging, setDragging] = useState(false)
  const { camera, size, gl, scene, invalidate } = useThree()
  const software = useMemo(() => {
    const context = gl.getContext()
    const debug = context.getExtension('WEBGL_debug_renderer_info')
    const renderer = context.getParameter(debug?.UNMASKED_RENDERER_WEBGL ?? context.RENDERER)
    return /swiftshader|llvmpipe|softpipe|software rasterizer|basic render driver|\bwarp\b/i.test(String(renderer))
  }, [gl])
  useEffect(() => {
    const previous = gl.info.autoReset
    gl.info.autoReset = false
    return () => { gl.info.autoReset = previous }
  }, [gl])
  const targetCamera = useRef(new Vector3(0, 0, 16))
  const targetLook = useRef(new Vector3(.4, 0, 0))
  const transitioning = useRef(true)
  const motion = settings.reducedMotion || background ? 0 : settings.motion
  const lastMetrics = useRef({ time: -1, frames: 0, intervals: [] as number[] })
  const glow = useMemo(() => glowTexture(), [])
  useEffect(() => () => glow.dispose(), [glow])
  const [visible, setVisible] = useState<Set<string> | null>(null)
  const projection = useMemo(() => new Vector3(), [])
  const metricsFramesToWait = useRef(1)
  useEffect(() => {
    // Demand rendering may stop before a three-second telemetry window elapses.
    // Publish actual geometry on the next rendered frame after relevant changes.
    lastMetrics.current = { time: -1, frames: 0, intervals: [] }
    metricsFramesToWait.current = 1
    invalidate()
  }, [page, nodes, background, settings.quality, settings.density, settings.nebula, settings.bloom, settings.reducedMotion, invalidate])
  const parallax = useRef({ x: 0, y: 0 })
  const actualPositions = useMemo(() => Object.fromEntries(nodes.map((node) => [node.id, page === 'World Tree' ? positions[node.id] ?? node.position : node.position])), [nodes, page, positions])
  const connections = useMemo(() => spatialConnections(nodes, actualPositions, page === 'World Tree'), [nodes, actualPositions, page])
  const activePath = useMemo(() => {
    const parents = new Map(nodes.map((node) => [node.id, node.parentId]))
    const path = new Set<string>()
    let id: string | undefined = selectedId
    while (id && parents.has(id) && !path.has(id)) { path.add(id); id = parents.get(id) }
    return path
  }, [nodes, selectedId])
  const rendered = nodes.length <= 64 ? nodes : nodes.filter((node) => node.kind === 'core' || activePath.has(node.id) || visible?.has(node.id))
  const fitPositions = useRef(actualPositions)
  useEffect(() => { fitPositions.current = actualPositions }, [actualPositions])
  const denseSelection = nodes.length > 64 ? nodes.find((node) => node.id === selectedId && node.kind === 'capability') : undefined
  useEffect(() => {
    const bounds = nodes.map((node) => fitPositions.current[node.id])
    const width = bounds.length ? Math.max(...bounds.map((p) => p[0])) - Math.min(...bounds.map((p) => p[0])) + 4 : 12
    const height = bounds.length ? Math.max(...bounds.map((p) => p[1])) - Math.min(...bounds.map((p) => p[1])) + 3 : 8
    const fitDistance = Math.max(orbitSizes[page], height / (focus ? .7 : .59), width / (size.width / size.height * (focus ? .85 : .66)) / .65) / zoom
    const distance = Math.min(denseSelection ? 24 : 58, fitDistance)
    const centerX = bounds.length ? (Math.max(...bounds.map((p) => p[0])) + Math.min(...bounds.map((p) => p[0]))) / 2 : -2
    const treeOverview = page === 'World Tree' && nodes.every((node) => node.kind === 'core' || node.kind === 'domain')
    const sideBias = treeOverview ? .43 : page === 'World Tree' ? .29 : .33
    const sideOffset = size.width < 760 ? 0 : distance * Math.tan(43 * Math.PI / 360) * size.width / size.height * sideBias
    const selectedPosition = denseSelection ? fitPositions.current[denseSelection.id] : undefined
    const anchorX = selectedPosition?.[0] ?? centerX
    targetLook.current.set(focus ? anchorX : anchorX + sideOffset, selectedPosition?.[1] ?? (focus ? 0 : .65), 0)
    targetCamera.current.set(targetLook.current.x, targetLook.current.y, distance)
    transitioning.current = true
  }, [page, fitToken, focus, zoom, nodes, denseSelection, size.width, size.height])
  useFrame(({ clock, pointer }, delta) => {
    // Sample the previous complete frame, including all composer passes, then
    // reset before this frame renders. The default auto-reset only counts the
    // final bloom/output pass and incorrectly reports one draw call.
    const previousCalls = gl.info.render.calls
    const previousTriangles = gl.info.render.triangles
    gl.info.reset()
    if (transitioning.current && controls.current) {
      const blend = motion ? 1 - Math.exp(-Math.min(delta, .35) * 6) : 1
      camera.position.lerp(targetCamera.current, blend)
      controls.current.target.lerp(targetLook.current, blend)
      controls.current.update()
      if (camera.position.distanceTo(targetCamera.current) < .015) transitioning.current = false
    } else if (motion && !dragging && controls.current) {
      const nextX = pointer.x * .12 * motion
      const nextY = pointer.y * .065 * motion
      camera.position.x += nextX - parallax.current.x
      camera.position.y += nextY - parallax.current.y
      parallax.current = { x: nextX, y: nextY }
    }
    if (nodes.length > 64 && (lastMetrics.current.frames % 6 === 0 || !visible)) {
      camera.updateMatrixWorld()
      const next = new Set<string>()
      for (const node of nodes) {
        projection.set(...actualPositions[node.id]).project(camera)
        if (Math.abs(projection.x) < 1.22 && Math.abs(projection.y) < 1.12 && Math.abs(projection.z) < 1) next.add(node.id)
      }
      if (next.size !== visible?.size || [...next].some((id) => !visible?.has(id))) setVisible(next)
    }
    lastMetrics.current.frames++
    if (delta > 0 && delta < 1) lastMetrics.current.intervals.push(delta * 1000)
    if (metricsFramesToWait.current > 0) {
      // Observe a completed volume/composer pass, rather than an unrendered FBO.
      metricsFramesToWait.current--
      invalidate()
    } else if (lastMetrics.current.time < 0 || clock.elapsedTime - lastMetrics.current.time >= 3) {
      let renderedStars = 0
      for (const name of ['drei-distant-stars', 'drei-middle-stars']) scene.getObjectByName(name)?.traverse((object) => { if (object instanceof Points) renderedStars += object.geometry.getAttribute('position').count })
      const sparkleObject = scene.getObjectByName('drei-spatial-sparkles') as Points | undefined
      const renderedSparkles = sparkleObject?.geometry?.getAttribute('position')?.count ?? 0
      const intervals = lastMetrics.current.intervals.sort((a, b) => a - b)
      onMetrics({ software, fps: lastMetrics.current.time < 0 ? 0 : Math.round(lastMetrics.current.frames / (clock.elapsedTime - lastMetrics.current.time)), p95: Math.round(intervals[Math.max(0, Math.ceil(intervals.length * .95) - 1)] ?? 0), calls: previousCalls, triangles: previousTriangles, geometries: gl.info.memory.geometries, textures: gl.info.memory.textures, stars: renderedStars, sparkles: renderedSparkles, renderer: gl.getContext().getParameter(gl.getContext().VERSION), cosmic: inspectCosmicEnvironment(scene,camera,size), bloom: settings.quality !== 'low' && settings.bloom > 0 })
      lastMetrics.current = { time: clock.elapsedTime, frames: 0, intervals: [] }
    }
  }, -3)
  return <>
    <color attach="background" args={['#09152c']} />
    <CosmicEnvironment page={background ? 'Mission Control' : page} quality={settings.quality} density={settings.density} motion={motion} nebula={settings.nebula} stars={settings.stars} particles={settings.particles}/>
    <ambientLight intensity={.55} color="#a0b7e0" /><directionalLight position={[-7, 8, 10]} intensity={3.2} color="#c8dfff" /><pointLight position={[3, -2, 2]} intensity={18} color="#5f59c7" distance={25} decay={2} />
    <Environment resolution={64} frames={1}><Lightformer position={[-4, 5, 5]} scale={[8, 2, 1]} intensity={2} color="#a4ccff" /><Lightformer position={[5, -2, 3]} scale={[2, 6, 1]} intensity={1.2} color="#8c7edc" /></Environment>
    <Suspense fallback={null}>
      {!background && rendered.map((node) => node.kind === 'core' ? <NeuralCore key={node.id} page={page} quality={settings.quality} texture={glow} node={{ ...node, position: actualPositions[node.id] }} motion={motion} onSelect={onSelect} selected={selectedId === node.id} pulseToken={pulseToken} caption={caption} /> : <SpatialOrb key={node.id} node={node} texture={glow} selected={selectedId === node.id} motion={motion} targetPosition={actualPositions[node.id]} focus={focus} onSelect={onSelect} onMove={onMove} onDrag={setDragging} movable={page === 'World Tree'} columnar={page === 'World Tree' || node.id.startsWith('function:')} pulseToken={pulseToken} low={settings.quality === 'low'} orbLabels={orbLabels} />)}
    </Suspense>
    {!background && <NeuralConnections edges={connections} active={activePath} selectedId={selectedId} motion={motion} token={pulseToken} />}
    <OrbitControls ref={controls} makeDefault enabled={!dragging} enableRotate enableDamping dampingFactor={.08} enablePan screenSpacePanning mouseButtons={{ LEFT: MOUSE.PAN, MIDDLE: MOUSE.DOLLY, RIGHT: MOUSE.ROTATE }} touches={{ ONE: TOUCH.PAN, TWO: TOUCH.DOLLY_ROTATE }} minDistance={7} maxDistance={65} minPolarAngle={page === 'World Tree' ? Math.PI / 2 : Math.PI * .35} maxPolarAngle={page === 'World Tree' ? Math.PI / 2 : Math.PI * .65} minAzimuthAngle={page === 'World Tree' ? -.2 : -Infinity} maxAzimuthAngle={page === 'World Tree' ? .2 : Infinity} onStart={() => { transitioning.current = false }} onEnd={() => { lastMetrics.current.time = -1; metricsFramesToWait.current = 1; invalidate() }} rotateSpeed={.25} zoomSpeed={.5} />
    {settings.quality !== 'low' && <EffectComposer multisampling={0}>
      {settings.bloom > 0 && <Bloom intensity={settings.bloom * (settings.quality === 'high' ? 1 : .65)} luminanceThreshold={1.3} luminanceSmoothing={.15} mipmapBlur />}
      <ToneMapping mode={ToneMappingMode.ACES_FILMIC}/>
    </EffectComposer>}
  </>
}
class RenderBoundary extends Component<{ children: ReactNode; fallback: ReactNode }, { failed: boolean }> {
  state = { failed: false }
  static getDerivedStateFromError() { return { failed: true } }
  render() { return this.state.failed ? this.props.fallback : this.props.children }
}
export default function UniverseEngine(props: Props) {
  const [hidden, setHidden] = useState(document.hidden)
  const [contextLost, setContextLost] = useState(false)
  const [minimized, setMinimized] = useState(false)
  const caption = useRef<HTMLDivElement>(null)
  // Keep Drei portal children outside the Canvas host that R3F can reconnect.
  const orbLabels = useRef<HTMLDivElement>(null!)
  const core = props.nodes.find((node) => node.kind === 'core')
  useEffect(() => { const listener = () => setHidden(document.hidden); document.addEventListener('visibilitychange', listener); return () => document.removeEventListener('visibilitychange', listener) }, [])
  useEffect(() => {
    if (!isTauri()) return
    let nativeWindow: ReturnType<typeof getCurrentWindow>
    try { nativeWindow = getCurrentWindow() } catch { return }
    let active = true
    let timer: number | undefined
    // WebView2 can stay document-visible when Windows minimizes its host.
    async function sampleWindow() {
      try {
        const value = await nativeWindow.isMinimized()
        if (active) setMinimized(value)
      } catch { return }
      if (active) timer = window.setTimeout(() => void sampleWindow(), 700)
    }
    void sampleWindow()
    return () => { active = false; window.clearTimeout(timer) }
  }, [])
  const fallback = <UniverseFallback nodes={props.nodes} onSelect={props.onSelect} />
  const frameloop = contextLost || hidden || minimized ? 'never' : props.background || props.settings.reducedMotion || props.settings.motion === 0 ? 'demand' : 'always'
  return <RenderBoundary fallback={fallback}>
    <div ref={orbLabels} className="universe-orb-labels" style={{ visibility: contextLost ? 'hidden' : undefined }} aria-hidden={contextLost || undefined} />
    <Canvas className="universe-renderer" style={{ visibility: contextLost ? 'hidden' : undefined }} aria-hidden={contextLost || undefined} aria-label="Interactive 3D capability universe" data-testid="universe-canvas" data-render-loop={frameloop} data-context-lost={contextLost} camera={{ position: [0, 0, 16], fov: 43, near: .1, far: 220 }} dpr={props.settings.quality === 'low' ? 1 : props.settings.quality === 'high' ? [1, 1.75] : [1, 1.35]} gl={{ antialias: props.settings.quality !== 'low', alpha: false, powerPreference: 'high-performance' }} frameloop={frameloop}><ContextEvents onLoss={setContextLost} /><Scene {...props} caption={caption} orbLabels={orbLabels} /></Canvas>
    {!contextLost && !props.background && core && <div className="core-caption" ref={caption}><button className="universe-core-label" onClick={() => props.onSelect(core)} aria-pressed={props.selectedId === core.id}><strong>{core.name}</strong><small>{core.subtitle}</small></button></div>}
    {contextLost && fallback}
  </RenderBoundary>
}
