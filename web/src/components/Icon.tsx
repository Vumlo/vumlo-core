import type { SVGProps } from "react";

/** Stroke paths on a 24px grid, taken from the design canvas. */
export const paths = {
  search: "M10.5 4a6.5 6.5 0 1 1 0 13 6.5 6.5 0 0 1 0-13zM15.5 15.5 20 20",
  moon: "M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z",
  sun: "M12 8a4 4 0 1 1 0 8 4 4 0 0 1 0-8zM12 2.5v2M12 19.5v2M2.5 12h2M19.5 12h2M5.3 5.3l1.4 1.4M17.3 17.3l1.4 1.4M5.3 18.7l1.4-1.4M17.3 6.7l1.4-1.4",
  lecture: "M3.5 5.5c2.5-.9 5.5-.7 8.5 1.2v13c-3-1.9-6-2.1-8.5-1.2zM20.5 5.5c-2.5-.9-5.5-.7-8.5 1.2v13c3-1.9 6-2.1 8.5-1.2z",
  assignment: "M13.5 4.5H6A1.5 1.5 0 0 0 4.5 6v12A1.5 1.5 0 0 0 6 19.5h12a1.5 1.5 0 0 0 1.5-1.5v-7.5M17.5 3.5l3 3L12 15l-3.5.5.5-3.5z",
  home: "M4 10.5 12 4l8 6.5V19a1 1 0 0 1-1 1h-4.5v-5.5h-5V20H5a1 1 0 0 1-1-1z",
  courses: "M4 6.5 12 3l8 3.5-8 3.5zM4 12l8 3.5 8-3.5M4 17.5 12 21l8-3.5",
  calendar: "M5 5.5h14a1 1 0 0 1 1 1V19a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V6.5a1 1 0 0 1 1-1zM4 10h16M8.5 3.5v4M15.5 3.5v4",
  grades: "M5 20V11M12 20V5M19 20v-6",
  grading: "M5 12.5l4.5 4.5L19 7.5M4 5h9",
  admin: "M12 3 4.5 6v5.5c0 4.4 3 7.6 7.5 9 4.5-1.4 7.5-4.6 7.5-9V6zM9 12l2.2 2.2L15.5 10",
  chevronRight: "M9 5l7 7-7 7",
  chevronLeft: "M15 5l-7 7 7 7",
  chevronDown: "M6 9l6 6 6-6",
  chevronUp: "M6 15l6-6 6 6",
  close: "M6 6l12 12M18 6 6 18",
  check: "M5 12.5l4.5 4.5L19 7.5",
  plus: "M12 5v14M5 12h14",
  tune: "M4 7h10M18 7h2M4 17h4M12 17h8M16 5v4M10 15v4",
  alert: "M12 7.5v5.5M12 16.5v.01",
  globe: "M3.5 12h17M12 3.5c2.3 2.4 3.4 5.2 3.4 8.5s-1.1 6.1-3.4 8.5c-2.3-2.4-3.4-5.2-3.4-8.5S9.7 5.9 12 3.5z",
  more: "M5 12h.01M12 12h.01M19 12h.01",
  logout: "M14 4.5H6.5A1.5 1.5 0 0 0 5 6v12a1.5 1.5 0 0 0 1.5 1.5H14M10 12h10M17 8.5 20.5 12 17 15.5",
  user: "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM4.5 20c.8-3.6 3.6-5.5 7.5-5.5s6.7 1.9 7.5 5.5",
  link: "M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3A4 4 0 0 0 11 18.7l1-1",
  file: "M14 3.5H7A1.5 1.5 0 0 0 5.5 5v14A1.5 1.5 0 0 0 7 20.5h10a1.5 1.5 0 0 0 1.5-1.5V8zM14 3.5V8h4.5",
  upload: "M12 16V5M7.5 9.5 12 5l4.5 4.5M5 19.5h14",
  trash: "M5 7h14M10 7V4.5h4V7M7 7l.8 12h8.4L17 7M10 11v5M14 11v5",
  edit: "M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4",
  eye: "M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12zM12 14.5a2.5 2.5 0 1 0 0-5 2.5 2.5 0 0 0 0 5z",
  grip: "M9 6h.01M15 6h.01M9 12h.01M15 12h.01M9 18h.01M15 18h.01",
  lock: "M7 11V8a5 5 0 0 1 10 0v3M6 11h12v9H6z",
  clock: "M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17zM12 7.5V12l3 2",
  download: "M12 4.5v11M7.5 11 12 15.5 16.5 11M5 19.5h14",
  video: "M4 6.5h11a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1zM16 10.5l5-3v9l-5-3",
  quiz: "M9.5 9a2.5 2.5 0 1 1 3.6 2.2c-.7.4-1.1 1-1.1 1.8M12 16.5v.01M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17z",
  form: "M6 4.5h12a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1v-13a1 1 0 0 1 1-1zM8.5 9h7M8.5 12.5h7M8.5 16h4",
  inClass: "M4 5.5h16v10H4zM8 19.5h8M12 15.5v4",
  menu: "M4 7h16M4 12h16M4 17h16",
  copy: "M9 9h10v11H9zM5 15V4.5h10",
  text: "M5 6.5h14M5 11h14M5 15.5h9",
  filter: "M4 6h16l-6 7.5V19l-4-2v-3.5z",
  arrowUp: "M12 19V5M6.5 10.5 12 5l5.5 5.5",
  arrowDown: "M12 5v14M6.5 13.5 12 19l5.5-5.5",
  refresh: "M20 11a8 8 0 0 0-14.5-3.5M4 4v4h4M4 13a8 8 0 0 0 14.5 3.5M20 20v-4h-4",
  focus: "M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5",
  focusExit: "M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5",
  info: "M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17zM12 11v5.5M12 7.5v.01",
} as const;

export type IconName = keyof typeof paths;

type Props = { name: IconName; size?: number } & Omit<SVGProps<SVGSVGElement>, "name">;

export function Icon({ name, size = 18, strokeWidth = 1.8, ...rest }: Props) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      {...rest}
    >
      <path d={paths[name]} />
    </svg>
  );
}
