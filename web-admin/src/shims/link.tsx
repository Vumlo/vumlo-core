import type { AnchorHTMLAttributes, MouseEvent } from "react";
import { navigate } from "../router";

/** `next/link` for the console: an anchor that moves within the page unless the browser should do it. */
export default function Link({ href, onClick, ...rest }: AnchorHTMLAttributes<HTMLAnchorElement> & { href: string }) {
  function click(e: MouseEvent<HTMLAnchorElement>) {
    onClick?.(e);
    const plain = e.button === 0 && !e.metaKey && !e.ctrlKey && !e.shiftKey && !e.altKey && !e.defaultPrevented;
    if (!plain || rest.target === "_blank" || /^[a-z]+:/i.test(href)) return;
    e.preventDefault();
    navigate(href);
  }
  return <a href={href} onClick={click} {...rest} />;
}
