import { createContext, useContext, useState, useCallback, useRef, type ReactNode } from "react";

interface Toast {
  id: number;
  message: string;
  type: "success" | "error" | "info";
}

interface ToastContextValue {
  toast: (message: string, type?: "success" | "error" | "info") => void;
  success: (message: string) => void;
  error: (message: string) => void;
  info: (message: string) => void;
}

const ToastContext = createContext<ToastContextValue>({
  toast: () => {},
  success: () => {},
  error: () => {},
  info: () => {},
});

export function useToast() {
  return useContext(ToastContext);
}

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const nextId = useRef(0);

  const push = useCallback(
    (message: string, type: "success" | "error" | "info" = "info") => {
      nextId.current += 1;
      const id = nextId.current;
      setToasts((prev) => [...prev, { id, message, type }]);
      setTimeout(() => {
        setToasts((prev) => prev.filter((t) => t.id !== id));
      }, 3000);
    },
    [],
  );

  const value: ToastContextValue = {
    toast: push,
    success: (m: string) => push(m, "success"),
    error: (m: string) => push(m, "error"),
    info: (m: string) => push(m, "info"),
  };

  return (
    <ToastContext.Provider value={value}>
      {children}
      <div className="toast-container" role="status" aria-live="polite" aria-atomic="false">
        {toasts.map((t) => (
          <div key={t.id} className={`toast toast-${t.type}`} role={t.type === "error" ? "alert" : undefined}>
            {t.message}
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}
