export { ThemeProvider, useTheme } from "./components/theme-provider"

// The shadcn UI components
export { Badge } from "./components/ui/badge"
export { Button } from "./components/ui/button"

// Icon surface for workspace consumers (lucide-react is a dep of this
// package, so apps import glyphs through the barrel instead of relying
// on undeclared/hoisted node_modules).
export {
  ArrowLeftIcon,
  CameraIcon,
  CheckIcon,
  ChevronRightIcon,
  GripVerticalIcon,
  InfoIcon,
  KeyboardIcon,
  KeyRoundIcon,
  MicIcon,
  PanelTopIcon,
  RotateCcwIcon,
  SettingsIcon,
  ShieldIcon,
  ShieldAlertIcon,
  ShieldCheckIcon,
  SlidersHorizontalIcon,
  XIcon,
} from "lucide-react"
