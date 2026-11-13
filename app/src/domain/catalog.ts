import type { ManagedInstallation } from '../data/installationManager'
import { BookOpen, FileText, Network, Sparkles, Workflow, type LucideIcon } from 'lucide-react'
import snapshot from './discovery-catalog.json'

export const branches = ['Engineering', 'Knowledge', 'Automation', 'Frontend'] as const
export type Category = typeof branches[number] | 'Unclassified'


export interface CatalogItem {
  managedInstallation?: ManagedInstallation
  id: string
  name: string
  owner: string
  repo: string
  githubUrl: string
  summary: string
  category: Category
  /** Optional real taxonomy path below the primary domain; empty means a direct leaf. */
  treePath?: string[]
  capabilities: string[]
  license: string
  mark: string
  Icon: LucideIcon
  kind?: 'skill' | 'plugin' | 'mcp' | 'repository'
  functionCategories?: string[]
  sourcePath?: string
  sourceRef?: string
  sourceBlobSha?: string
  repositoryStars?: number
  repositoryPushedAt?: string
  repositoryArchived?: boolean
  verifiedAt?: string
  compatibility?: string
  compatibilityEvidence?: string
  setupRequirements?: string[]
  bundle?: string
  bundledSkillCount?: number
  qualityNote?: string
  github?: {
    repository?: import('../data/githubDiscovery').DiscoveryState['repositories'][number]
    assessment?: import('../data/githubDiscovery').Intelligence
    classification?: import('../data/githubDiscovery').Classification
  }
}

// These are curated repository candidates, not installed Codex skills.
// License evidence comes from the GitHub repository API snapshot in docs/technology-research.md.
const legacyCatalog: CatalogItem[] = [
  {
    id: 'github-mcp', name: 'GitHub MCP', owner: 'github', repo: 'github-mcp-server',
    githubUrl: 'https://github.com/github/github-mcp-server',
    summary: 'GitHub’s official MCP server for connecting an agent to GitHub context and workflows.',
    category: 'Engineering', capabilities: ['Repository context', 'Issue workflows'], license: 'MIT', mark: 'github', Icon: Network,
  },
  {
    id: 'context7', name: 'Context7', owner: 'upstash', repo: 'context7',
    githubUrl: 'https://github.com/upstash/context7',
    summary: 'A documentation platform that provides current library references for AI agents.',
    category: 'Knowledge', capabilities: ['Library documentation', 'Source lookup'], license: 'MIT', mark: 'context', Icon: BookOpen,
  },
  {
    id: 'playwright-mcp', name: 'Playwright MCP', owner: 'microsoft', repo: 'playwright-mcp',
    githubUrl: 'https://github.com/microsoft/playwright-mcp',
    summary: 'An MCP server that exposes browser automation through Playwright.',
    category: 'Automation', capabilities: ['Browser automation', 'Page inspection'], license: 'Apache-2.0', mark: 'playwright', Icon: Workflow,
  },
  {
    id: 'markitdown', name: 'MarkItDown', owner: 'microsoft', repo: 'markitdown',
    githubUrl: 'https://github.com/microsoft/markitdown',
    summary: 'A Python tool for converting files and office documents to Markdown.',
    category: 'Knowledge', capabilities: ['Document conversion', 'Markdown extraction'], license: 'MIT', mark: 'markitdown', Icon: FileText,
  },
  {
    id: 'superpowers', name: 'Superpowers', owner: 'obra', repo: 'superpowers',
    githubUrl: 'https://github.com/obra/superpowers',
    summary: 'An agentic skills framework and software development methodology.',
    category: 'Engineering', capabilities: ['Development workflows', 'Reusable skills'], license: 'MIT', mark: 'superpowers', Icon: Sparkles,
  },
  {
    id: 'vercel-agent-skills', name: 'Vercel Agent Skills', owner: 'vercel-labs', repo: 'agent-skills',
    githubUrl: 'https://github.com/vercel-labs/agent-skills',
    summary: 'Vercel’s collection of agent skills. Repository license has not been verified.',
    category: 'Frontend', capabilities: ['Frontend guidance', 'Reusable skills'], license: 'License not verified', mark: 'vercel', Icon: Network,
  },
]

export const discoveryCategories = snapshot.categories
export const discoveryVerifiedAt = snapshot.verifiedAt
export const retiredCatalogIds = new Set(['superpowers', 'vercel-agent-skills'])
export const catalog: CatalogItem[] = [
  ...snapshot.items.map((entry): CatalogItem => ({ ...entry, category: entry.category as Category, kind: entry.kind as 'skill' | 'plugin', mark: entry.kind, Icon: entry.kind === 'plugin' ? Sparkles : BookOpen })),
  ...legacyCatalog.filter((entry) => !retiredCatalogIds.has(entry.id)).map((entry): CatalogItem => ({ ...entry,
    kind: entry.id === 'markitdown' ? 'repository' : 'mcp',
    functionCategories: [entry.id === 'github-mcp' ? 'development' : entry.id === 'context7' ? 'research' : entry.id === 'playwright-mcp' ? 'browser' : 'documents'],
  })),
]
