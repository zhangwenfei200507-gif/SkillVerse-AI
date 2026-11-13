# Planet texture assets

Downloaded from the NASA Jet Propulsion Laboratory Solar System Simulator texture map library on 2026-10-30. Maps are bundled locally so the app does not fetch images at runtime.

| File | Source map | Notes |
|---|---|---|
| `earth.jpg` | [JPL Earth map](https://space.jpl.nasa.gov/tmaps/earth.html) | USGS/JPL/Caltech land and ocean/topographic coloring. |
| `venus.jpg` | [JPL Venus map](https://space.jpl.nasa.gov/tmaps/venus.html) | Planetary map from the JPL texture library. |
| `mars.jpg` | [JPL Mars map](https://space.jpl.nasa.gov/tmaps/mars.html) | Viking imagery processed by USGS. |
| `jupiter.jpg` | [JPL Jupiter map](https://space.jpl.nasa.gov/tmaps/jupiter.html) | Voyager map, explicitly described as badly processed. |
| `saturn.jpg` | [JPL Saturn map](https://space.jpl.nasa.gov/tmaps/saturn.html) | Representative color map; ring geometry is rendered separately. |
| `neptune.jpg` | [JPL Neptune map](https://space.jpl.nasa.gov/tmaps/neptune.html) | Representative gas-giant appearance map. |

JPL's [texture-map library](https://space.jpl.nasa.gov/tmaps/) explains that some maps are spacecraft mosaics, may have gaps or aesthetic processing, and are not scientific-analysis data. It also states gas-giant maps represent an appearance that changes with atmospheric dynamics. These assets are visual textures, not scientific data. Attribution/source references are retained here.

## Deep-space backdrop

- `app/public/assets/space/hubble-ultra-deep-field.webp` is the NASA-hosted image [Hubble Ultra Deep Field](https://science.nasa.gov/image-detail/hubble_ultra_deep_field_stsci-prc04-07a/), image credit: NASA, ESA, S. Beckwith (STScI), and the HUDF Team. NASA's page identifies it as a 2239 × 2233 image and describes the galaxies in the field. It is bundled as a low-opacity far-background image layer under the app's own stars and planets.
