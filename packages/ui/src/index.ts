export { Button, buttonVariants } from "./components/ui/button"
export { cn } from "./lib/utils"
export { ThemeProvider, useTheme } from "./components/theme-provider"

// Icon surface for workspace consumers (lucide-react is a dep of this
// package, so apps import glyphs through the barrel instead of relying
// on undeclared/hoisted node_modules).
export {
  ArrowLeft,
  Camera,
  Check,
  ChevronRight,
  Fingerprint,
  Info,
  Keyboard,
  KeyRound,
  Lock,
  Mic,
  RotateCcw,
  Settings,
  Shield,
  ShieldAlert,
  ShieldCheck,
  SlidersHorizontal,
  X,
} from "lucide-react"
