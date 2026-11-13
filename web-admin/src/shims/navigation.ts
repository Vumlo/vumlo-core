import { navigate, useLocation } from "../router";

// What the code shared with the student app asks of `next/navigation`, answered by the console's router.

export function useRouter() {
  return {
    push: (to: string) => navigate(to),
    replace: (to: string) => navigate(to, true),
    back: () => window.history.back(),
  };
}

export const usePathname = () => useLocation().path;

export const useSearchParams = () => new URLSearchParams(useLocation().search);
