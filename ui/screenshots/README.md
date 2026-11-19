# SkillVerse UI screenshots

These captures document the redesigned workspace at 1440 × 900. They were captured from the Vite browser preview with the repository's demonstration catalog. The top-right `Browser preview` badge is visible in the captures.

The browser preview does not scan local Codex files, write to the desktop database, install capabilities, or execute Mission Control plans. Screens showing local inventory and workflow state are explicitly empty or fixture-backed. The Mission Control planning fixture is a UI fixture, not proof of a connected Codex runtime.

## Workspace pages

| Page or state | Screenshot | Notes |
| --- | --- | --- |
| World Tree | [Capability atlas](world-tree.png) | Full 20-category atlas, catalog rail, and inspector. |
| World Tree | [Compact atlas](world-tree-compact.png) | Dense view with the inspector closed. |
| World Tree | [Selected capability](world-tree-inspector.png) | Capability details and task actions in the right inspector. |
| Discover | [Directory](discover.png) | Category-led source catalog and discovery status. |
| Discover | [Atlas view](discover-atlas.png) | Catalog categories arranged as an interactive atlas. |
| Discover | [Selected source](discover-inspector.png) | Repository/source details in the inspector. |
| Installed | [Local inventory](installed.png) | Browser-preview state clearly marks the local scan as unavailable. |
| Learning | [Learning workspace](learning.png) | Learning path before a capability is selected. |
| Learning | [Capability practice](learning-capability.png) | Selected capability and staged practice view. |
| Settings | [Display](settings-display.png) | Rendering, motion, and visual controls. |
| Settings | [Local Codex](settings-local-codex.png) | Local scan and data controls. |
| Settings | [AI Providers](settings-ai-providers.png) | Provider configuration surface. |
| Settings | [Privacy](settings-privacy.png) | Privacy and local-data controls. |
| First run | [Getting started](getting-started.png) | Initial setup and capability orientation. |
| Mission Control | [Browser boundary](mission-control.png) | Browser preview explains that local workflow execution requires the desktop app. |
| Mission Control | [Planning fixture](mission-control-planning-fixture.png) | Synthetic plan for reviewing layout and step inspector behavior. |

## Design references and validation

- [xyflow/xyflow](https://github.com/xyflow/xyflow), MIT, `@xyflow/react` 12.12.0 in the existing lockfile. Its canvas examples and viewport controls informed the atlas navigation and fit-to-view affordances; the app keeps its existing Three.js scene and does not copy upstream components. See the [React examples](https://github.com/xyflow/xyflow/tree/main/examples/react) and [release history](https://github.com/xyflow/xyflow/releases).
- [shadcn/ui](https://github.com/shadcn-ui/ui), MIT, reviewed at the current 4.23.0 release. Its documented alert and progressive-disclosure patterns informed clearly labeled settings, inspector details, and explicit preview boundaries. No package or upstream component code was added. See the [component documentation](https://ui.shadcn.com/docs/components) and [release history](https://github.com/shadcn-ui/ui/releases).

Validation run for this redesign: `npm run build` and the project's Vitest suite; browser preview captured all listed pages and states. The gallery records visual coverage only; native scanning, installation, and runtime execution require their desktop verification path.
