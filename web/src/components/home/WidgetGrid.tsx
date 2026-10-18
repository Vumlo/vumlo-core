"use client";

import { useEffect, useRef, useState, type ReactNode } from "react";
import { Icon } from "@/components/Icon";
import type { MessageKey } from "@/lib/i18n/core";
import { useT } from "@/lib/i18n";
import s from "./WidgetGrid.module.css";

export type WidgetDef = {
  id: string;
  title: string;
  desc: string;
  /** Sizes as "WxH" in grid cells; the first is the default. */
  sizes: string[];
  link?: ReactNode;
  note?: string;
  badge?: number | null;
  /** False hides the widget on phones. */
  phone?: boolean;
  render: (w: number, h: number) => ReactNode;
};

type Layout = { order: string[]; sizes: Record<string, string> };

const SIZE_NAMES: Record<string, MessageKey> = { "1x1": "wg.small", "2x1": "wg.wide", "1x2": "wg.tall", "2x2": "wg.large", "4x1": "wg.full" };
const label = (z: string) => z.replace("x", "×");

function readLayout(key: string): Partial<Layout> | null {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as Partial<Layout>) : null;
  } catch {
    return null;
  }
}

/**
 * The home page grid. In customize mode widgets can be dragged or moved with the arrows, resized,
 * removed and added back. The layout is kept in this browser only (Home design: "saved in this browser").
 */
export function WidgetGrid({ storageKey, defs, defaultOrder, customizeSlot }: { storageKey: string; defs: WidgetDef[]; defaultOrder: string[]; customizeSlot?: (button: ReactNode) => ReactNode }) {
  const t = useT();
  const byId = new Map(defs.map((d) => [d.id, d]));
  const ids = useRef(defs.map((d) => d.id));
  const [layout, setLayout] = useState<Layout>({ order: defaultOrder, sizes: {} });
  const [customizing, setCustomizing] = useState(false);
  const [menuFor, setMenuFor] = useState<string | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);

  useEffect(() => {
    const saved = readLayout(storageKey);
    if (saved?.order) {
      setLayout({ order: saved.order.filter((id) => ids.current.includes(id)), sizes: saved.sizes ?? {} });
    }
  }, [storageKey]);

  function update(next: Layout) {
    setLayout(next);
    setMenuFor(null);
    try {
      localStorage.setItem(storageKey, JSON.stringify(next));
    } catch {
      // Storage can be blocked; the layout lasts for this visit.
    }
  }

  const sizeOf = (d: WidgetDef) => {
    const chosen = layout.sizes[d.id];
    return chosen && d.sizes.includes(chosen) ? chosen : d.sizes[0]!;
  };

  const move = (id: string, delta: number) => {
    const o = layout.order.slice();
    const i = o.indexOf(id);
    const j = i + delta;
    if (j < 0 || j >= o.length) return;
    o.splice(i, 1);
    o.splice(j, 0, id);
    update({ ...layout, order: o });
  };
  const moveTo = (id: string | null, target: string) => {
    setDragId(null);
    if (!id || id === target) return;
    const o = layout.order.filter((x) => x !== id);
    o.splice(o.indexOf(target), 0, id);
    update({ ...layout, order: o });
  };

  const shown = layout.order.map((id) => byId.get(id)).filter((d): d is WidgetDef => !!d);
  const hidden = defs.filter((d) => !layout.order.includes(d.id));
  const area = shown.reduce((a, d) => {
    const [w, h] = sizeOf(d).split("x").map(Number);
    return a + (w ?? 1) * (h ?? 1);
  }, 0);
  const guides = (Math.max(3, Math.ceil(area / 4)) + 1) * 4;

  const button = !customizing && (
    <button type="button" className="btn" onClick={() => setCustomizing(true)}>
      <Icon name="tune" size={17} />
      {t("home.customize")}
    </button>
  );

  return (
    <>
      {customizeSlot ? customizeSlot(button) : button}
      {customizing && (
        <div className={s.banner}>
          <p>
            <strong>{t("home.customizeTitle")}</strong> {t("home.customizeLead")}
          </p>
          <button type="button" className="btn btn-outline" onClick={() => update({ order: defaultOrder, sizes: {} })}>{t("home.resetDefault")}</button>
          <button type="button" className="btn btn-primary" onClick={() => { setCustomizing(false); setMenuFor(null); }}>{t("common.done")}</button>
        </div>
      )}
      <div className={s.stack}>
        {customizing && (
          <div aria-hidden="true" className={s.guides}>
            {Array.from({ length: guides }, (_, i) => <div key={i} className={s.guide} />)}
          </div>
        )}
        <div className={s.grid}>
          {shown.map((d, i) => {
            const size = sizeOf(d);
            const [w, h] = size.split("x").map(Number) as [number, number];
            const menuOpen = customizing && menuFor === d.id;
            return (
              <section
                key={d.id}
                aria-label={d.title}
                className={s.widget}
                data-w={w}
                data-h={h}
                data-phone={d.phone === false ? "false" : undefined}
                data-editing={customizing}
                data-dragging={customizing && dragId === d.id}
                data-menu={menuOpen}
                draggable={customizing}
                onDragStart={(e) => { e.dataTransfer.effectAllowed = "move"; e.dataTransfer.setData("text/plain", d.id); setDragId(d.id); }}
                onDragOver={(e) => { if (customizing) e.preventDefault(); }}
                onDrop={(e) => { e.preventDefault(); moveTo(e.dataTransfer.getData("text/plain") || dragId, d.id); }}
                onDragEnd={() => setDragId(null)}
              >
                <div className={s.head}>
                  {customizing && (
                    <span title={t("cm.drag")} className={s.handle}>
                      <Icon name="grip" size={16} strokeWidth={3} />
                    </span>
                  )}
                  <h2 className={s.title}>
                    {d.title}
                    {d.badge ? <span className={s.badge}>{d.badge}</span> : null}
                  </h2>
                  {!customizing && d.link}
                  {!customizing && d.note && <span className={s.note}>{d.note}</span>}
                  {customizing && (
                    <div className={s.tools}>
                      {d.sizes.length > 1 ? (
                        <button type="button" className={s.sizeBtn} aria-haspopup="menu" aria-expanded={menuOpen} aria-label={t("wg.size", { size: label(size) })} onClick={() => setMenuFor(menuOpen ? null : d.id)}>
                          {label(size)}
                          <Icon name="chevronDown" size={14} strokeWidth={2} />
                        </button>
                      ) : (
                        <span className={s.fixed} title={t("wg.oneSize")}>{label(size)}</span>
                      )}
                      <button type="button" className={s.tool} aria-label={t("wg.earlier", { title: d.title })} disabled={i === 0} onClick={() => move(d.id, -1)}>
                        <Icon name="chevronLeft" size={16} strokeWidth={2} />
                      </button>
                      <button type="button" className={s.tool} aria-label={t("wg.later", { title: d.title })} disabled={i === shown.length - 1} onClick={() => move(d.id, 1)}>
                        <Icon name="chevronRight" size={16} strokeWidth={2} />
                      </button>
                      <button type="button" className={s.tool} aria-label={t("wg.remove", { title: d.title })} style={{ color: "var(--muted)" }} onClick={() => update({ ...layout, order: layout.order.filter((x) => x !== d.id) })}>
                        <Icon name="close" size={16} strokeWidth={2} />
                      </button>
                    </div>
                  )}
                </div>
                {menuOpen && (
                  <div role="menu" aria-label={t("wg.widgetSize")} className={`popover ${s.menu}`}>
                    {d.sizes.map((z) => (
                      <button
                        key={z}
                        type="button"
                        role="menuitemradio"
                        aria-checked={z === size}
                        className="menu-item"
                        onClick={() => update({ ...layout, sizes: { ...layout.sizes, [d.id]: z } })}
                      >
                        <Icon name="check" size={16} strokeWidth={2.2} style={{ color: "var(--accent-fg)", opacity: z === size ? 1 : 0 }} />
                        <span style={{ flex: "1 1 auto" }}>{SIZE_NAMES[z] ? t(SIZE_NAMES[z]!) : z}</span>
                        <span className="muted">{label(z)}</span>
                      </button>
                    ))}
                  </div>
                )}
                <div className={s.body} data-editing={customizing}>{d.render(w, h)}</div>
              </section>
            );
          })}
        </div>
      </div>
      {customizing && (
        <section aria-labelledby="more-h" className={s.more} style={{ marginTop: 32 }}>
          <h2 id="more-h">{t("home.moreWidgets")}</h2>
          <p className="muted" style={{ margin: "0 2px 12px", fontSize: 14 }}>{t("home.moreLead")}</p>
          {hidden.length > 0 ? (
            <div className={s.moreGrid}>
              {hidden.map((d) => (
                <div key={d.id} className={s.moreCard}>
                  <div className="row" style={{ justifyContent: "space-between", alignItems: "baseline", gap: 8 }}>
                    <span style={{ fontSize: 15, fontWeight: 600 }}>{d.title}</span>
                    <span className="muted" style={{ fontSize: 13 }}>{label(sizeOf(d))}</span>
                  </div>
                  <p className="muted" style={{ fontSize: 13, margin: "0 0 10px" }}>{d.desc}</p>
                  <button type="button" className="btn btn-outline" style={{ marginTop: "auto" }} onClick={() => update({ ...layout, order: [...layout.order, d.id] })}>{t("home.addToHome")}</button>
                </div>
              ))}
            </div>
          ) : (
            <p style={{ margin: "0 2px", fontSize: 14 }}>{t("wg.allOn")}</p>
          )}
        </section>
      )}
    </>
  );
}
