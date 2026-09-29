# AGENTS

You're the topest full-stack web engineer, especially in NextJS framework and related severless deployment. You will follow the principles below

## Core Principles

### 1. Package Management: Use bun

- **Always** use `bun` for package management
- Install: `bun install <package>`
- Run scripts: `bun run <script>`
- Dev dependencies: `bun add -D <package>`
- Production dependencies: `bun add <package>`

### 2. Component Syntax: Arrow Functions Only

- **Always** use arrow function syntax for React components, hooks, and context providers
- ✅ `const MyComponent = () => { ... }`
- ❌ `function MyComponent() { ... }`
- Scoped to authored code — vendored shadcn files in `packages/ui/src/components/ui/` keep upstream style

### 3. Export Pattern: Named Exports Only (Non-UI Components)

- **Always** use named exports for components in `src/components/*`
- ✅ `export const MyComponent = () => { ... }`
- ❌ `export default MyComponent`

### 4. Code Reuse: Refactor Repeated Components

- Extract repeated UI patterns into reusable components
- Create shared components in `src/components/` directories
- Use composition over duplication

### 5. Discovery: Find from Current Project First

- Search existing codebase before creating new patterns
- Check `src/components/`, hooks, and utilities for existing solutions

### 6. Markdown Documentation: Compact Table Style

- **Always** use compact table format with spaces after opening pipes
- ✅ `| --- | --- |` (space after `|`)
- ❌ `|---|---|` (no space)

### 7. Page Files: Keep as Server Components

- `page.tsx` files **must remain Server Components** by default — do **not** add `"use client"` to a page file
- Move client-side interactivity into dedicated child components (with `"use client"`) and import them into the page

### 8. Shared UI components

- Use `packages/ui` for shared UI components, it's Shadcn's standard UI components
- Shared different in apps/

### 9. Icons: `Icon`-Suffixed lucide-react Names Only

- **Always** use the `Icon`-suffixed lucide-react alias when exporting or importing icons — never the bare name
- ✅ `export { XIcon, InfoIcon } from "lucide-react"` / `import { XIcon } from '@marvis/ui'`
- ❌ `export { X, Info } from "lucide-react"` / `import { X } from '@marvis/ui'`
- Apps import icons through the `@marvis/ui` barrel (`packages/ui/src/index.ts`) — keep its export list suffixed too

## Project Structure

<!-- BEGIN:nextjs-agent-rules -->

## This is NOT the Next.js you know

This version has breaking changes — APIs, conventions, and file structure may all differ from your training data. Read the relevant guide in `node_modules/next/dist/docs/` (resolved from this file's directory; in monorepos the `next` package may not be visible from the repo root) before writing any code. Heed deprecation notices.

This block is written and re-added by `next dev` — verify at `node_modules/next/dist/server/lib/generate-agent-files.js`. Removing it from a diff only re-creates the uncommitted change; committing it with your work keeps the tree clean.

<!-- END:nextjs-agent-rules -->