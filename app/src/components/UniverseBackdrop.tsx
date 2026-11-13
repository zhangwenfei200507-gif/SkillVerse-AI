import { useEffect, useRef } from 'react'

type UniverseBackdropProps = { page: string }

const PAGE_CAMERAS: Record<string, [number, number, number]> = {
  'World Tree': [0, 0, 15],
  Discover: [-2.2, 0.25, 14],
  Installed: [2.2, 0, 14],
  Learning: [0.4, -1.25, 14],
  Settings: [0, 1.15, 14],
}
const PAGE_ORBITS: Record<string, [number, number, number]> = {
  'World Tree': [-0.8, 0, 0], Discover: [0.1, -0.35, 0], Installed: [0.8, 0.1, 0],
  Learning: [-6.8, -0.55, 0], Settings: [1.1, 0.2, 0],
}

export function UniverseBackdrop({ page }: UniverseBackdropProps) {
  const hostRef = useRef<HTMLDivElement>(null)
  const pageRef = useRef(page)
  useEffect(() => { pageRef.current = page }, [page])

  useEffect(() => {
    const host = hostRef.current
    if (!host || /jsdom/i.test(navigator.userAgent)) return
    let cancelled = false
    let raf = 0
    let renderer: import('three').WebGLRenderer | undefined
    let scene: import('three').Scene | undefined
    let camera: import('three').PerspectiveCamera | undefined
    let removeListeners = () => {}

    void import('three').then(async (THREE) => {
      if (cancelled || !host) return
      try {
        const canvas = document.createElement('canvas')
        canvas.className = 'universe-canvas'
        host.append(canvas)
        const gl = canvas.getContext('webgl2', { alpha: true, antialias: false, powerPreference: 'low-power', depth: true, stencil: false })
        if (!gl) { host.dataset.fallback = 'true'; return }

        const activeScene = new THREE.Scene()
        activeScene.background = new THREE.Color(0x030815)
        activeScene.fog = new THREE.FogExp2(0x071328, 0.014)
        const activeCamera = new THREE.PerspectiveCamera(48, 1, 0.1, 90)
        activeCamera.position.set(0, 0, 15)
        const activeRenderer = new THREE.WebGLRenderer({ canvas, context: gl, alpha: false, antialias: false, powerPreference: 'low-power' })
        renderer = activeRenderer
        scene = activeScene
        camera = activeCamera
        activeRenderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 1.25))
        activeRenderer.setClearColor(0x030815, 1)
        activeRenderer.outputColorSpace = THREE.SRGBColorSpace

        activeScene.add(new THREE.HemisphereLight(0x8ac8ff, 0x071020, 1.15))
        const blueLight = new THREE.PointLight(0x2a83ff, 95, 38, 1.7)
        blueLight.position.set(-5, 3, 1)
        activeScene.add(blueLight)
        const violetLight = new THREE.PointLight(0x7551ff, 72, 34, 1.8)
        violetLight.position.set(6, -3, -4)
        activeScene.add(violetLight)

        const nebulaMaterial = new THREE.ShaderMaterial({
          transparent: true, depthWrite: false, side: THREE.DoubleSide,
          uniforms: { uTime: { value: 0 }, uTintA: { value: new THREE.Color(0x1d4eac) }, uTintB: { value: new THREE.Color(0x63358f) } },
          vertexShader: `varying vec2 vUv; void main(){vUv=uv;gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.0);}`,
          fragmentShader: `varying vec2 vUv; uniform float uTime; uniform vec3 uTintA; uniform vec3 uTintB; float hash(vec2 p){return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453);} float noise(vec2 p){vec2 i=floor(p),f=fract(p);f=f*f*(3.0-2.0*f);return mix(mix(hash(i),hash(i+vec2(1.0,0.0)),f.x),mix(hash(i+vec2(0.0,1.0)),hash(i+vec2(1.0,1.0)),f.x),f.y);} void main(){vec2 p=vUv;float n=noise(p*5.0+vec2(uTime*0.018,-uTime*0.011))*0.55+noise(p*11.0-vec2(uTime*0.009))*0.28+noise(p*23.0)*0.17;float band=exp(-pow((p.y-(0.42+0.13*sin(p.x*5.2+uTime*0.025)))*3.5,2.0));float mask=smoothstep(0.18,0.75,n)*band;float haze=exp(-length((p-vec2(0.26,0.56))*vec2(1.1,1.8))*3.0)*0.48+exp(-length((p-vec2(0.76,0.34))*vec2(1.4,2.1))*3.4)*0.38;vec3 c=mix(uTintA,uTintB,smoothstep(0.22,0.82,p.x+0.12*sin(uTime*0.025)));gl_FragColor=vec4(c,mask*0.55+haze*(0.5+0.04*sin(uTime*0.08)));}`,
        })
        const nebula = new THREE.Mesh(new THREE.PlaneGeometry(56, 32), nebulaMaterial)
        nebula.position.set(0, 0, -27)
        activeScene.add(nebula)

        let seed = 91357
        const random = () => { seed = (seed * 16807) % 2147483647; return (seed - 1) / 2147483646 }
        const starCount = window.innerWidth < 700 ? 620 : 1480
        const positions = new Float32Array(starCount * 3)
        const colors = new Float32Array(starCount * 3)
        const sizes = new Float32Array(starCount)
        const cool = new THREE.Color(0x95c7ff)
        const warm = new THREE.Color(0xffd5a1)
        for (let i = 0; i < starCount; i += 1) {
          const radius = 14 + random() * 38
          const theta = random() * Math.PI * 2
          const z = -3 - random() * 48
          positions[i * 3] = Math.cos(theta) * radius
          positions[i * 3 + 1] = Math.sin(theta) * radius * 0.62
          positions[i * 3 + 2] = z
          const color = random() > 0.91 ? warm : cool
          const isBeacon = random() > 0.983
          const brightness = isBeacon ? 2.1 + random() * 1.2 : 0.68 + random() * 0.68
          colors[i * 3] = color.r * brightness
          colors[i * 3 + 1] = color.g * brightness
          colors[i * 3 + 2] = color.b * brightness
          sizes[i] = isBeacon ? 0.82 + random() * 0.82 : 0.12 + random() * 0.27
        }
        const starGeometry = new THREE.BufferGeometry()
        starGeometry.setAttribute('position', new THREE.BufferAttribute(positions, 3))
        starGeometry.setAttribute('color', new THREE.BufferAttribute(colors, 3))
        starGeometry.setAttribute('aSize', new THREE.BufferAttribute(sizes, 1))
        const starMaterial = new THREE.ShaderMaterial({
          transparent: true, depthWrite: false, vertexColors: true,
          vertexShader: `attribute float aSize; varying vec3 vColor; void main(){vColor=color;vec4 mv=modelViewMatrix*vec4(position,1.0);gl_PointSize=aSize*min(155.0/max(1.0,-mv.z),2.5);gl_Position=projectionMatrix*mv;}`,
          fragmentShader: `varying vec3 vColor; void main(){float d=length(gl_PointCoord-0.5);float halo=1.0-smoothstep(0.04,0.5,d);float core=1.0-smoothstep(0.0,0.13,d);gl_FragColor=vec4(mix(vColor,vec3(0.86,0.95,1.0),core*0.62),halo*0.8);}`,
        })
        const stars = new THREE.Points(starGeometry, starMaterial)
        activeScene.add(stars)

        const orbitGroup = new THREE.Group()
        activeScene.add(orbitGroup)
        const orbitSpecs = [
          { x: -12, y: 5.5, z: -22, radius: 1.45, color: 0x8dc9ff, texture: 'mars', atmosphere: 0x579dff },
          { x: 11.5, y: 5.2, z: -26, radius: 1.55, color: 0xb4a9ff, texture: 'jupiter', atmosphere: 0x8479ff },
          { x: -13, y: -6, z: -24, radius: 1.25, color: 0x65dfff, texture: 'earth', atmosphere: 0x3fbfff },
          { x: 13, y: -5.5, z: -30, radius: 1.8, color: 0x8d9dff, texture: 'saturn', atmosphere: 0x9285ff, ring: true },
          { x: -16, y: -1.5, z: -34, radius: 0.9, color: 0xe7b78e, texture: 'venus', atmosphere: 0xffbd83 },
          { x: 6, y: 8.7, z: -38, radius: 0.95, color: 0x78bfff, texture: 'neptune', atmosphere: 0x559bff },
        ]
        const textureLoader = new THREE.TextureLoader()
        const globeTextures = await Promise.all(orbitSpecs.map(async (spec) => {
          try {
            const texture = await textureLoader.loadAsync(`/assets/planets/${spec.texture}.jpg`)
            texture.colorSpace = THREE.SRGBColorSpace
            texture.wrapS = THREE.RepeatWrapping
            texture.anisotropy = Math.min(activeRenderer.capabilities.getMaxAnisotropy(), 8)
            return texture
          } catch {
            return undefined
          }
        }))
        let deepFieldTexture: import('three').Texture | undefined
        try {
          deepFieldTexture = await textureLoader.loadAsync('/assets/space/hubble-ultra-deep-field.webp')
          deepFieldTexture.colorSpace = THREE.SRGBColorSpace
        } catch {
          // Keep the local starfield and nebula shader as a complete offline fallback.
        }
        if (cancelled || !host) {
          globeTextures.forEach((texture) => texture?.dispose())
          deepFieldTexture?.dispose()
          activeScene.clear()
          activeRenderer.dispose()
          activeRenderer.forceContextLoss()
          return
        }
        const keyLight = new THREE.DirectionalLight(0xdcecff, 3.1)
        keyLight.position.set(-8, 6, 13)
        activeScene.add(keyLight)
        if (deepFieldTexture) {
          const deepField = new THREE.Mesh(new THREE.PlaneGeometry(120, 70), new THREE.ShaderMaterial({
            transparent: true, depthWrite: false, blending: THREE.AdditiveBlending,
            uniforms: { uMap: { value: deepFieldTexture } },
            vertexShader: `varying vec2 vUv; void main(){vUv=uv;gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.0);}`,
            fragmentShader: `varying vec2 vUv; uniform sampler2D uMap; void main(){vec2 uv=vec2(vUv.x,(vUv.y-.5)*.58+.5);vec3 color=texture2D(uMap,uv).rgb;float light=dot(color,vec3(.2126,.7152,.0722));float visibility=smoothstep(.018,.13,light);gl_FragColor=vec4(color,visibility*.44);}`,
          }))
          deepField.position.set(0, 0, -60)
          activeScene.add(deepField)
        }
        const globeMaterials = orbitSpecs.map((spec, index) => new THREE.MeshStandardMaterial({ color: globeTextures[index] ? 0xffffff : spec.color, map: globeTextures[index], roughness: 0.92, metalness: 0.01 }))
        const atmosphereMaterials = orbitSpecs.map((spec) => new THREE.ShaderMaterial({
          transparent: true, depthWrite: false, side: THREE.FrontSide, blending: THREE.AdditiveBlending,
          uniforms: { uAtmosphere: { value: new THREE.Color(spec.atmosphere) } },
          vertexShader: `varying vec3 vNormal; varying vec3 vView; void main(){vec4 mv=modelViewMatrix*vec4(position,1.0);vNormal=normalize(normalMatrix*normal);vView=normalize(-mv.xyz);gl_Position=projectionMatrix*mv;}`,
          fragmentShader: `varying vec3 vNormal; varying vec3 vView; uniform vec3 uAtmosphere; void main(){float facing=max(dot(normalize(vNormal),normalize(vView)),0.0);float rim=pow(1.0-facing,3.4);gl_FragColor=vec4(uAtmosphere, rim*.18);}`,
        }))
        const orbitalMeshes: import('three').Group[] = []
        for (const [index, spec] of orbitSpecs.entries()) {
          const body = new THREE.Group()
          body.position.set(spec.x, spec.y, spec.z)
          const globe = new THREE.Mesh(new THREE.SphereGeometry(spec.radius, 72, 56), globeMaterials[index])
          body.add(globe)
          body.add(new THREE.Mesh(new THREE.SphereGeometry(spec.radius * 1.055, 48, 36), atmosphereMaterials[index]))
          if (spec.ring) {
            const rings = new THREE.Group()
            const bands = [{ inner: 1.28, outer: 1.47, opacity: 0.26, color: 0x857b76 }, { inner: 1.49, outer: 1.72, opacity: 0.62, color: 0xd7c3a7 }, { inner: 1.74, outer: 1.9, opacity: 0.23, color: 0x9a8c7c }]
            for (const band of bands) {
              rings.add(new THREE.Mesh(new THREE.RingGeometry(spec.radius * band.inner, spec.radius * band.outer, 128), new THREE.MeshStandardMaterial({ color: band.color, side: THREE.DoubleSide, transparent: true, opacity: band.opacity, roughness: 0.95, metalness: 0.02, depthWrite: false })))
            }
            rings.rotation.set(1.18, 0.05, -0.18)
            body.add(rings)
          }
          orbitalMeshes.push(body)
          orbitGroup.add(body)
        }

        const resize = () => {
          if (!host || !camera || !renderer) return
          const width = host.clientWidth || window.innerWidth
          const height = host.clientHeight || window.innerHeight
          camera.aspect = width / height
          camera.updateProjectionMatrix()
          renderer.setSize(width, height, false)
        }
        const pointer = { x: 0, y: 0 }
        const onPointer = (event: PointerEvent) => { pointer.x = (event.clientX / Math.max(window.innerWidth, 1) - 0.5) * 0.55; pointer.y = (event.clientY / Math.max(window.innerHeight, 1) - 0.5) * 0.32 }
        const observer = new ResizeObserver(resize)
        observer.observe(host)
        window.addEventListener('pointermove', onPointer, { passive: true })
        resize()

        const cameraNow = new THREE.Vector3(0, 0, 15)
        const cameraFrom = cameraNow.clone()
        const cameraTo = cameraNow.clone()
        const orbitNow = new THREE.Vector3(...(PAGE_ORBITS[pageRef.current] ?? PAGE_ORBITS['World Tree']))
        const orbitFrom = orbitNow.clone()
        const orbitTo = orbitNow.clone()
        let transitionStarted = performance.now()
        let currentPage = pageRef.current
        let lastFrame = 0
        const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)')
        const animate = (now: number) => {
          raf = window.requestAnimationFrame(animate)
          if (document.hidden || (now - lastFrame < (reducedMotion.matches ? 160 : 33))) return
          const dt = Math.min((now - (lastFrame || now)) / 1000, 0.08)
          lastFrame = now
          if (pageRef.current !== currentPage) {
            currentPage = pageRef.current
            cameraFrom.copy(cameraNow)
            cameraTo.set(...(PAGE_CAMERAS[currentPage] ?? PAGE_CAMERAS['World Tree']))
            orbitFrom.copy(orbitNow)
            orbitTo.set(...(PAGE_ORBITS[currentPage] ?? PAGE_ORBITS['World Tree']))
            transitionStarted = now
          }
          const progress = Math.min(1, Math.max(0, (now - transitionStarted) / 780))
          const eased = 1 - Math.pow(1 - progress, 3)
          cameraNow.lerpVectors(cameraFrom, cameraTo, eased)
          orbitNow.lerpVectors(orbitFrom, orbitTo, eased)
          orbitGroup.position.copy(orbitNow)
          activeCamera.position.x += (cameraNow.x + pointer.x - activeCamera.position.x) * Math.min(1, dt * 1.7)
          activeCamera.position.y += (cameraNow.y - pointer.y - activeCamera.position.y) * Math.min(1, dt * 1.7)
          activeCamera.position.z += (cameraNow.z - activeCamera.position.z) * Math.min(1, dt * 1.5)
          activeCamera.lookAt(cameraNow.x * 0.14, cameraNow.y * 0.14, -6)
          const drift = reducedMotion.matches ? 0 : now * 0.000012
          stars.rotation.y = drift
          nebulaMaterial.uniforms.uTime.value = reducedMotion.matches ? 0 : now * 0.001
          orbitGroup.rotation.y = drift * 0.8
          orbitalMeshes.forEach((body, index) => { body.rotation.y = drift * (index % 2 ? -0.5 : 0.8); body.rotation.z = Math.sin(now * 0.00013 + index) * 0.018 })
          activeRenderer.render(activeScene, activeCamera)
        }
        raf = window.requestAnimationFrame(animate)
        removeListeners = () => {
          observer.disconnect()
          window.removeEventListener('pointermove', onPointer)
          window.cancelAnimationFrame(raf)
          activeScene.traverse((object) => {
            const renderable = object as import('three').Mesh
            renderable.geometry?.dispose()
            const materials = renderable.material
            if (Array.isArray(materials)) materials.forEach((material) => material.dispose())
            else materials?.dispose()
          })
          globeTextures.forEach((texture) => texture?.dispose())
          deepFieldTexture?.dispose()
          activeRenderer.dispose()
          activeRenderer.forceContextLoss()
        }
      } catch {
        host.dataset.fallback = 'true'
      }
    })

    return () => {
      cancelled = true
      removeListeners()
      if (!renderer && scene) scene.clear()
      camera = undefined
    }
  }, [])

  return <div className="universe-backdrop" ref={hostRef} aria-hidden="true"><div className="universe-vignette" /></div>
}
