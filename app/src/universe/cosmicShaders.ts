export const noise3D = `
float hash3(vec3 p) {
  p=fract(p*.1031); p+=dot(p,p.yzx+33.33);
  return fract((p.x+p.y)*p.z);
}
float noise3(vec3 p) {
  vec3 i=floor(p),f=fract(p);f=f*f*(3.-2.*f);
  return mix(mix(mix(hash3(i),hash3(i+vec3(1,0,0)),f.x),
    mix(hash3(i+vec3(0,1,0)),hash3(i+vec3(1,1,0)),f.x),f.y),
    mix(mix(hash3(i+vec3(0,0,1)),hash3(i+vec3(1,0,1)),f.x),
    mix(hash3(i+vec3(0,1,1)),hash3(i+vec3(1,1,1)),f.x),f.y),f.z);
}
float fbm(vec3 p) {
  return noise3(p)*.57+noise3(p*2.03+vec3(4.1,7.2,2.8))*.28+noise3(p*4.13+vec3(9.2,1.8,5.4))*.15;
}`

export const skyVertex = `
varying vec3 direction0;
void main(){direction0=position;gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.);}`
export const skyFragment = `
varying vec3 direction0;
uniform vec3 deepColor;uniform vec3 liftColor;uniform vec3 gasColor;
uniform float time;uniform float strength;
${noise3D}
void main(){
  vec3 d=normalize(direction0);
  float upper=smoothstep(-.6,.65,d.y);
  vec3 col=mix(deepColor,liftColor,.08+upper*.21);
  float ridge=exp(-pow((d.y-.09-.32*sin(d.x*2.4+d.z))*4.8,2.));
  #if FAST_SKY
    float dust=noise3(d*9.0);
    float detail=.55;
  #else
    float dust=fbm(d*9.0+vec3(time*.002,0.,0.));
    float detail=fbm(d*23.+vec3(2.,5.,1.));
  #endif
  float wisps=smoothstep(.36,.78,dust)*ridge*(.45+detail);
  col+=gasColor*wisps*.32*strength;
  // A distant, soft galaxy has structure in direction space, not a wallpaper.
  float hub=exp(-dot((d.xy-vec2(.42,.26))*vec2(5.,8.),(d.xy-vec2(.42,.26))*vec2(5.,8.)));
  col+=liftColor*hub*.4*strength;
  gl_FragColor=vec4(col,1.);
  #include <tonemapping_fragment>
  #include <colorspace_fragment>
}`

export const nebulaVertex = `
varying vec3 localSurface;
void main(){localSurface=position;gl_Position=projectionMatrix*modelViewMatrix*vec4(position,1.);}`
export const nebulaFragment = `
varying vec3 localSurface;
uniform mat4 worldToLocal;
uniform float time;uniform float strength;
uniform vec3 gasColor;uniform vec3 accentColor;
${noise3D}
float ellipsoid(vec3 p,vec3 center,vec3 extent){vec3 q=(p-center)/extent;return exp(-dot(q,q)*2.3);}
float densityAt(vec3 p){
  float shape=ellipsoid(p,vec3(-.29,.18,-.08),vec3(.37,.27,.34))
    +ellipsoid(p,vec3(.3,.19,.12),vec3(.32,.2,.32))*.85
    +ellipsoid(p,vec3(.02,-.24,.22),vec3(.45,.2,.28))*.48;
  vec3 flow=vec3(sin(p.y*6.+time*.012),cos(p.x*7.-time*.009),sin(p.z*5.))*.065;
  vec3 frequency=vec3(15.,19.,4.);
  float gas=fbm((p+flow)*frequency+vec3(time*.003,-time*.002,0.));
  return shape*pow(smoothstep(.38,.74,gas),1.5)*3.1;
}
void main(){
  vec3 origin=(worldToLocal*vec4(cameraPosition,1.)).xyz;
  vec3 direction=normalize(localSurface-origin);
  vec3 inv=1./direction;
  vec3 a=(-vec3(.5)-origin)*inv,b=(vec3(.5)-origin)*inv;
  vec3 lo=min(a,b),hi=max(a,b);
  float nearT=max(max(lo.x,lo.y),lo.z),farT=min(min(hi.x,hi.y),hi.z);
  nearT=max(nearT,0.);
  if(farT<=nearT)discard;
  float stepSize=(farT-nearT)/float(VOLUME_STEPS);
  float transmission=1.;vec3 emission=vec3(0.);
  for(int i=0;i<VOLUME_STEPS;i++){
    vec3 p=origin+direction*(nearT+(float(i)+.5)*stepSize);
    float density=densityAt(p);
    float opacity=1.-exp(-density*stepSize*2.4*strength);
    vec3 tint=mix(gasColor,accentColor,smoothstep(-.3,.4,p.x+p.z*.6));
    float filament=.32+noise3(p*vec3(40.,40.,8.)+vec3(0.,time*.003,0.))*1.25;
    emission+=transmission*opacity*tint*filament;
    transmission*=1.-opacity;
    if(transmission<.035)break;
  }
  float alpha=1.-transmission;
  gl_FragColor=vec4(emission/max(alpha,.001),alpha);
  #include <tonemapping_fragment>
  #include <colorspace_fragment>
}`

export const luminousVertex = `
attribute float starSize;attribute float phase;attribute float twinkleSpeed;
uniform float time;uniform float pixelRatio;
varying vec3 tint;varying float brightness;
void main(){
  vec4 p=modelViewMatrix*vec4(position,1.);
  tint=color;brightness=.82+.18*sin(time*twinkleSpeed+phase);
  gl_PointSize=clamp(starSize*pixelRatio*30./max(8.,-p.z),1.,28.);
  gl_Position=projectionMatrix*p;
}`
export const luminousFragment = `
varying vec3 tint;varying float brightness;uniform float opacity;
void main(){
  vec2 p=gl_PointCoord-.5;float r=length(p);
  float core=exp(-r*r*150.);
  float halo=exp(-r*r*25.)*.23;
  float rays=(exp(-abs(p.x)*130.)+exp(-abs(p.y)*130.))*exp(-r*13.)*.08;
  gl_FragColor=vec4(tint*brightness,clamp(core+halo+rays,0.,1.)*opacity);
  #include <tonemapping_fragment>
  #include <colorspace_fragment>
}`
