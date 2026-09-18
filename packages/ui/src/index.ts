export { Button, buttonVariants } from "./components/ui/button"
export { cn } from "./lib/utils"
export { ThemeProvider, useTheme } from "./components/theme-provider"

// Icon surface for workspace consumers (lucide-react is a dep of this
// package, so apps import glyphs through the barrel instead of relying
// on undeclared/hoisted node_modules).
export { Mic, Settings, ShieldAlert } from "lucide-react"
