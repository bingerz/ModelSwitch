// Centralized icon exports so the dashboard never imports lucide-react
// directly. This makes it trivial to swap icon libraries later.
//
// Note: this project pins lucide-react@^1.21 which uses the new naming
// convention (CircleCheck, TriangleAlert, CircleAlert) instead of the
// legacy aliases (CheckCircle2, AlertTriangle, AlertCircle).
export {
  Activity,
  Server,
  CircleCheck,
  CircleAlert,
  TriangleAlert,
  CircleOff,
  CircleX,
  Zap,
  DollarSign,
  Cpu,
  Timer,
  TrendingUp,
  TrendingDown,
  Radio,
  ArrowUpRight,
  RefreshCw,
  Loader,
  Inbox,
} from "lucide-react";
