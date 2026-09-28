export { cn } from "./lib/utils"
export { ThemeProvider, useTheme } from "./components/theme-provider"

// The shadcn UI components
export { Badge } from "./components/ui/badge"
export { Button } from "./components/ui/button"
export {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  DialogTrigger,
} from "./components/ui/dialog"
export { Input } from "./components/ui/input"
export { Label } from "./components/ui/label"
export { ShineBorder } from "./components/ui/shine-border"
export { Tabs, TabsContent, TabsList, TabsTrigger } from "./components/ui/tabs"
export { ToggleGroup, ToggleGroupItem } from "./components/ui/toggle-group"

// Icon surface for workspace consumers (lucide-react is a dep of this
// package, so apps import glyphs through the barrel instead of relying
// on undeclared/hoisted node_modules).
export {
  AppWindowIcon,
  ArrowLeftIcon,
  CameraIcon,
  CheckIcon,
  ChevronDownIcon,
  ChevronRightIcon,
  ChevronUpIcon,
  CopyIcon,
  EllipsisIcon,
  GripVerticalIcon,
  HistoryIcon,
  InfoIcon,
  KeyboardIcon,
  KeyRoundIcon,
  LayoutGridIcon,
  MessageSquareTextIcon,
  MicIcon,
  MicAudioLinesIcon,
  MonitorDotIcon,
  MonitorIcon,
  PanelTopIcon,
  PauseIcon,
  PlayIcon,
  RotateCcwIcon,
  SettingsIcon,
  ShieldIcon,
  ShieldAlertIcon,
  ShieldCheckIcon,
  SlidersHorizontalIcon,
  SquareIcon,
  Trash2Icon,
  VideoIcon,
  XIcon,
} from "lucide-react"
