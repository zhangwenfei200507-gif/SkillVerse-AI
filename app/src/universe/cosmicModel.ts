import { Color } from 'three'
import type { ScenePage } from './model'

export type CosmicQuality = 'low' | 'balanced' | 'high'
export type UniverseTheme = ScenePage | 'Mission Control'
export const cosmicThemes: Record<UniverseTheme, { deep: string; lift: string; gas: string; accent: string; strength: number }> = {
  'World Tree': { deep: '#061225', lift: '#153c65', gas: '#376aa4', accent: '#5e518d', strength: .73 },
  Discover: { deep: '#0b1129', lift: '#292b58', gas: '#4b67b4', accent: '#7c4ca0', strength: .91 },
  Installed: { deep: '#061924', lift: '#0c354a', gas: '#28758a', accent: '#436796', strength: .71 },
  Learning: { deep: '#0d112b', lift: '#222d58', gas: '#4969a8', accent: '#875c9c', strength: .83 },
  Settings: { deep: '#081324', lift: '#153047', gas: '#32657e', accent: '#56618e', strength: .45 },
  'Mission Control': { deep: '#06121e', lift: '#163746', gas: '#286a85', accent: '#5368a0', strength: .69 },
}
export function cosmicBudget(quality: CosmicQuality, density: number, compact = false) {
  const scale = compact ? .65 : 1
  return {
    distant: Math.round((quality === 'low' ? 2600 : 5600) * density * scale),
    middle: Math.round((quality === 'low' ? 420 : 900) * density * scale),
    luminous: Math.round((quality === 'low' ? 110 : 240) * density * scale),
    foreground: Math.round((quality === 'low' ? 8 : 18) * density),
    steps: quality === 'low' ? 8 : quality === 'high' ? 20 : 12,
  }
}
// Stable positions preserve spatial identity during scene changes and make
// camera parallax measurable rather than confusing it with regenerated stars.
export function luminousStarData(count: number) {
  let seed = 78231
  const random = () => { seed = (Math.imul(1664525, seed) + 1013904223) >>> 0; return seed / 4294967296 }
  const position = new Float32Array(count * 3)
  const color = new Float32Array(count * 3)
  const size = new Float32Array(count)
  const phase = new Float32Array(count)
  const speed = new Float32Array(count)
  const temperatures = ['#bad9ff', '#d9eaff', '#b8b7ed', '#f2d8ae']
  const tint = new Color()
  for (let i = 0; i < count; i++) {
    position.set([(random() - .5) * 130, (random() - .5) * 85, -7 - random() * 88], i * 3)
    tint.set(temperatures[Math.floor(random() * temperatures.length)])
    // Only ~5% of this modest population can exceed the bloom threshold.
    tint.multiplyScalar(i % 23 === 0 ? 3.5 : .6 + random() * .3)
    color.set([tint.r, tint.g, tint.b], i * 3)
    size[i] = i % 23 === 0 ? 15 + random() * 5 : 3 + random() * 5
    phase[i] = random() * Math.PI * 2
    speed[i] = .16 + random() * .45
  }
  return { position, color, size, phase, speed }
}
export function distantGalaxyData(count: number) {
  let seed=9183
  const random=()=>{seed=(Math.imul(1664525,seed)+1013904223)>>>0;return seed/4294967296}
  const position=new Float32Array(count*3),color=new Float32Array(count*3),size=new Float32Array(count),phase=new Float32Array(count),speed=new Float32Array(count)
  const tint=new Color()
  for(let i=0;i<count;i++){
    const radius=Math.pow(random(),.72)*16
    const arm=i%3*Math.PI*2/3
    const angle=arm+radius*.34+(random()-.5)*.5
    const scatter=(random()-.5)*(1+radius*.13)
    position.set([Math.cos(angle)*radius+scatter,(random()-.5)*.7,Math.sin(angle)*radius+scatter],i*3)
    tint.set(i%7===0?'#c8b7e5':'#829fda').multiplyScalar(.42)
    color.set([tint.r,tint.g,tint.b],i*3);size[i]=3+random()*6;phase[i]=random()*6.28;speed[i]=.02
  }
  return {position,color,size,phase,speed}
}
