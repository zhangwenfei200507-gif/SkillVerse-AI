import { useEffect, useRef } from 'react'
import type { RefObject } from 'react'
import type { WebGLRenderer } from 'three'

type NeuralCore3DProps = {
  selected: boolean
  onReady: (ready: boolean) => void
  canvasRef: RefObject<HTMLCanvasElement | null>
}

function makeStarField(THREE: typeof import('three'), count: number) {
  const positions = new Float32Array(count * 3)
  let seed = 571
  const random = () => {
    seed = (seed * 16807) % 2147483647
    return (seed - 1) / 2147483646
  }

  for (let index = 0; index < count; index += 1) {
    const radius = 1.12 + random() * 1.25
    const theta = random() * Math.PI * 2
    const phi = Math.acos(2 * random() - 1)
    positions[index * 3] = radius * Math.sin(phi) * Math.cos(theta)
    positions[index * 3 + 1] = radius * Math.cos(phi)
    positions[index * 3 + 2] = radius * Math.sin(phi) * Math.sin(theta)
  }

  const geometry = new THREE.BufferGeometry()
  geometry.setAttribute('position', new THREE.BufferAttribute(positions, 3))
  return geometry
}

function disposeScene(scene: import('three').Scene) {
  scene.traverse((object) => {
    const mesh = object as import('three').Mesh
    mesh.geometry?.dispose()
    if (Array.isArray(mesh.material)) mesh.material.forEach((material) => material.dispose())
    else mesh.material?.dispose()
  })
}

export function NeuralCore3D({ selected, onReady, canvasRef }: NeuralCore3DProps) {
  const stageRef = useRef<HTMLDivElement>(null)
  const selectedRef = useRef(selected)
  const startPulseRef = useRef<(() => void) | null>(null)

  useEffect(() => {
    const previous = selectedRef.current
    selectedRef.current = selected
    if (previous !== selected) startPulseRef.current?.()
  }, [selected])

  useEffect(() => {
    const canvas = canvasRef.current
    const stage = stageRef.current
    if (!canvas || !stage || /jsdom/i.test(navigator.userAgent)) return

    let disposed = false
    let renderer: WebGLRenderer | undefined
    let sceneForCleanup: import('three').Scene | undefined
    let removeListeners = () => {}

    void import('three').then((THREE) => {
      if (disposed) return
      try {
        const context = canvas.getContext('webgl2', {
          alpha: true,
          antialias: true,
          depth: true,
          powerPreference: 'low-power',
          premultipliedAlpha: true,
          preserveDrawingBuffer: false,
          stencil: false,
        })
        if (!context) return
        const createdRenderer = new THREE.WebGLRenderer({
          canvas,
          context,
          alpha: true,
          antialias: true,
          powerPreference: 'low-power',
        })
        renderer = createdRenderer
        createdRenderer.setClearColor(0x000000, 0)
        createdRenderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.35))
        createdRenderer.outputColorSpace = THREE.SRGBColorSpace

        const scene = new THREE.Scene()
        sceneForCleanup = scene
        const camera = new THREE.PerspectiveCamera(33, 1, 0.1, 12)
        camera.position.set(0, 0, 3.65)

        scene.add(new THREE.AmbientLight(0x85aaff, 1.3))
        const keyLight = new THREE.PointLight(0x69dcff, 4.6, 7)
        keyLight.position.set(-1.15, 1.3, 2.2)
        scene.add(keyLight)
        const rimLight = new THREE.PointLight(0x8372ff, 3.3, 7)
        rimLight.position.set(1.7, -1.2, -1.5)
        scene.add(rimLight)

        const core = new THREE.Group()
        scene.add(core)

        const nucleus = new THREE.Mesh(
          new THREE.SphereGeometry(0.4, 40, 32),
          new THREE.MeshPhongMaterial({
            color: 0x367dd1,
            emissive: 0x102b68,
            emissiveIntensity: 0.55,
            shininess: 105,
            specular: 0xc3f2ff,
            transparent: true,
            opacity: 0.94,
          }),
        )
        core.add(nucleus)

        const volumeShell = new THREE.Mesh(
          new THREE.SphereGeometry(0.83, 40, 32),
          new THREE.MeshPhysicalMaterial({
            color: 0x4d9cff,
            emissive: 0x123b80,
            emissiveIntensity: 0.22,
            metalness: 0.12,
            roughness: 0.3,
            clearcoat: 0.82,
            clearcoatRoughness: 0.22,
            transparent: true,
            opacity: 0.17,
            depthWrite: false,
            side: THREE.DoubleSide,
          }),
        )
        core.add(volumeShell)

        const fresnelRim = new THREE.Mesh(
          new THREE.SphereGeometry(0.86, 40, 32),
          new THREE.ShaderMaterial({
            transparent: true,
            depthWrite: false,
            blending: THREE.AdditiveBlending,
            vertexShader: `varying vec3 vNormal; varying vec3 vViewDirection; void main() { vec4 viewPosition = modelViewMatrix * vec4(position, 1.0); vNormal = normalize(normalMatrix * normal); vViewDirection = normalize(-viewPosition.xyz); gl_Position = projectionMatrix * viewPosition; }`,
            fragmentShader: `varying vec3 vNormal; varying vec3 vViewDirection; void main() { float rim = pow(1.0 - max(dot(normalize(vNormal), normalize(vViewDirection)), 0.0), 2.25); vec3 blue = mix(vec3(0.16, 0.52, 1.0), vec3(0.5, 0.38, 1.0), smoothstep(-0.45, 0.8, vNormal.y)); gl_FragColor = vec4(blue, rim * 0.72); }`,
          }),
        )
        fresnelRim.renderOrder = 2
        core.add(fresnelRim)

        const innerLight = new THREE.Mesh(
          new THREE.SphereGeometry(0.23, 32, 24),
          new THREE.MeshBasicMaterial({ color: 0x86e6ff, transparent: true, opacity: 0.62 }),
        )
        innerLight.position.set(-0.11, 0.12, 0.27)
        core.add(innerLight)

        const shell = new THREE.Mesh(
          new THREE.IcosahedronGeometry(0.73, 2),
          new THREE.MeshBasicMaterial({ color: 0x799cff, wireframe: true, transparent: true, opacity: 0.15 }),
        )
        core.add(shell)

        const wireframe = new THREE.LineSegments(
          new THREE.EdgesGeometry(new THREE.IcosahedronGeometry(0.73, 1)),
          new THREE.LineBasicMaterial({ color: 0x9adfff, transparent: true, opacity: 0.48 }),
        )
        core.add(wireframe)

        const nodesGeometry = new THREE.IcosahedronGeometry(0.74, 1)
        const nodeMaterial = new THREE.PointsMaterial({
          color: 0xd8f7ff,
          size: 0.036,
          sizeAttenuation: true,
          transparent: true,
          opacity: 0.84,
        })
        core.add(new THREE.Points(nodesGeometry, nodeMaterial))

        const starsGeometry = makeStarField(THREE, window.innerWidth <= 580 ? 76 : 152)
        const starsMaterial = new THREE.PointsMaterial({
          color: 0xb3d4ff,
          size: 0.014,
          transparent: true,
          opacity: 0.55,
          sizeAttenuation: true,
        })
        scene.add(new THREE.Points(starsGeometry, starsMaterial))

        const rings = [
          { radius: 0.94, tube: 0.008, color: 0x79d9ff, rotation: [1.16, 0.2, 0.36] as const, opacity: 0.62 },
          { radius: 1.08, tube: 0.005, color: 0x7584ff, rotation: [0.38, 1.02, -0.48] as const, opacity: 0.43 },
          { radius: 1.22, tube: 0.0035, color: 0xc3adff, rotation: [1.72, -0.62, 0.74] as const, opacity: 0.28 },
        ]
        const orbitGroups: import('three').Group[] = []
        rings.forEach((ring, index) => {
          const orbit = new THREE.Group()
          const [rotationX, rotationY, rotationZ] = ring.rotation
          orbit.rotation.set(rotationX, rotationY, rotationZ)
          const geometry = new THREE.TorusGeometry(ring.radius, ring.tube, 5, 112)
          const material = new THREE.MeshBasicMaterial({
            color: ring.color,
            transparent: true,
            opacity: ring.opacity,
            depthWrite: false,
          })
          orbit.add(new THREE.Mesh(geometry, material))

          const satellite = new THREE.Mesh(
            new THREE.SphereGeometry(index === 0 ? 0.032 : 0.022, 12, 10),
            new THREE.MeshBasicMaterial({ color: index === 0 ? 0xdaf8ff : ring.color }),
          )
          satellite.position.set(ring.radius, 0, 0)
          orbit.add(satellite)
          core.add(orbit)
          orbitGroups.push(orbit)
        })

        const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)')
        let pointerX = 0
        let pointerY = 0
        let targetPointerX = 0
        let targetPointerY = 0
        let pulseStartedAt = 0
        let renderLoopActive = false
        let visible = true

        const setSize = () => {
          const bounds = stage.getBoundingClientRect()
          if (!bounds.width || !bounds.height) return
          createdRenderer.setSize(bounds.width, bounds.height, false)
          camera.aspect = bounds.width / bounds.height
          camera.updateProjectionMatrix()
          if (!renderLoopActive) createdRenderer.render(scene, camera)
        }

        const animate = (time: number) => {
          const elapsed = time - pulseStartedAt
          const pulseEnvelope = reducedMotion.matches || !pulseStartedAt
            ? 0
            : Math.exp(-Math.pow((elapsed - 155) / 105, 2)) * 0.9
              + Math.exp(-Math.pow((elapsed - 735) / 155, 2)) * 0.56
          pointerX += (targetPointerX - pointerX) * 0.075
          pointerY += (targetPointerY - pointerY) * 0.075
          core.rotation.y = pointerX * 0.2 + (pulseEnvelope * 0.035)
          core.rotation.x = -pointerY * 0.16
          core.scale.setScalar(1 + pulseEnvelope * 0.035)
          orbitGroups.forEach((orbit, index) => {
            orbit.rotation.z += reducedMotion.matches ? 0 : (index % 2 ? -1 : 1) * 0.0015 * (1 + pulseEnvelope)
          })
          createdRenderer.render(scene, camera)

          const settling = Math.abs(targetPointerX - pointerX) + Math.abs(targetPointerY - pointerY) > 0.006
          const pulsing = !reducedMotion.matches && time - pulseStartedAt < 1260
          if (!visible || (!settling && !pulsing)) {
            renderLoopActive = false
            core.rotation.y = pointerX * 0.2
            core.rotation.x = -pointerY * 0.16
            core.scale.setScalar(1)
            createdRenderer.render(scene, camera)
            createdRenderer.setAnimationLoop(null)
          }
        }

        const startMotion = () => {
          if (!visible || reducedMotion.matches || renderLoopActive) return
          renderLoopActive = true
          createdRenderer.setAnimationLoop(animate)
        }
        const pulse = () => {
          pulseStartedAt = performance.now()
          if (!reducedMotion.matches && visible) startMotion()
          else createdRenderer.render(scene, camera)
        }
        startPulseRef.current = pulse

        const onPointerMove = (event: PointerEvent) => {
          if (reducedMotion.matches) return
          const bounds = stage.getBoundingClientRect()
          targetPointerX = ((event.clientX - bounds.left) / bounds.width - 0.5) * 2
          targetPointerY = ((event.clientY - bounds.top) / bounds.height - 0.5) * 2
          startMotion()
        }
        const onPointerLeave = () => {
          targetPointerX = 0
          targetPointerY = 0
          startMotion()
        }
        const interactionTarget = stage.parentElement ?? stage
        interactionTarget.addEventListener('pointermove', onPointerMove, { passive: true })
        interactionTarget.addEventListener('pointerleave', onPointerLeave, { passive: true })

        const resizeObserver = new ResizeObserver(setSize)
        resizeObserver.observe(stage)
        const visibilityObserver = new IntersectionObserver(([entry]) => {
          visible = entry.isIntersecting && document.visibilityState === 'visible'
          if (!visible) {
            renderLoopActive = false
            createdRenderer.setAnimationLoop(null)
          } else createdRenderer.render(scene, camera)
        })
        visibilityObserver.observe(stage)
        const onVisibilityChange = () => {
          visible = document.visibilityState === 'visible'
          if (!visible) {
            renderLoopActive = false
            createdRenderer.setAnimationLoop(null)
          } else createdRenderer.render(scene, camera)
        }
        document.addEventListener('visibilitychange', onVisibilityChange)
        const onMotionPreference = () => {
          if (reducedMotion.matches) {
            renderLoopActive = false
            createdRenderer.setAnimationLoop(null)
            targetPointerX = 0
            targetPointerY = 0
            pointerX = 0
            pointerY = 0
            core.rotation.set(0, 0, 0)
            core.scale.setScalar(1)
            createdRenderer.render(scene, camera)
          }
        }
        reducedMotion.addEventListener('change', onMotionPreference)

        setSize()
        onReady(true)
        removeListeners = () => {
          interactionTarget.removeEventListener('pointermove', onPointerMove)
          interactionTarget.removeEventListener('pointerleave', onPointerLeave)
          document.removeEventListener('visibilitychange', onVisibilityChange)
          reducedMotion.removeEventListener('change', onMotionPreference)
          resizeObserver.disconnect()
          visibilityObserver.disconnect()
          startPulseRef.current = null
        }
      } catch {
        if (sceneForCleanup) disposeScene(sceneForCleanup)
        renderer?.dispose()
      }
    }).catch(() => {
      // Keep the SVG artwork visible if WebGL is unavailable or fails to initialize.
    })

    return () => {
      disposed = true
      removeListeners()
      if (renderer) {
        renderer.setAnimationLoop(null)
        if (sceneForCleanup) disposeScene(sceneForCleanup)
        renderer.dispose()
      }
      onReady(false)
    }
  }, [canvasRef, onReady])

  return <div ref={stageRef} className="core-webgl-stage" aria-hidden="true"><canvas ref={canvasRef} className="core-webgl-canvas" data-testid="neural-core-3d" /></div>
}
