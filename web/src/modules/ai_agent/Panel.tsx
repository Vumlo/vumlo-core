"use client";

import { useEffect, useRef, useState, type FormEvent, type KeyboardEvent } from "react";
import { Icon } from "@/components/Icon";
import { useT } from "@/lib/i18n";
import type { MessageKey } from "@/lib/i18n/core";
import { invalidate, useMutation } from "@/lib/query";
import { useMe } from "@/lib/session";
import { CHAT, clearBuddyChat, pageContext, sendBuddyMessage, useBuddyChat, type BuddyMessage, type Proposal } from "./api";
import { ProposalCard, type Settled } from "./ProposalCard";
import s from "./Buddy.module.css";

const NOTICE_KEY = "vumlo.buddy.notice";
const MAX_TEXT = 4000;

/** The notice is shown until the person has accepted this very text on this device. */
function noticeSeen(text: string) {
  try {
    return localStorage.getItem(NOTICE_KEY) === text;
  } catch {
    return false;
  }
}

function rememberNotice(text: string) {
  try {
    localStorage.setItem(NOTICE_KEY, text);
  } catch {
    // Storage can be blocked; the notice then comes back next time.
  }
}

/** The chat with the Buddy: as a side panel (`side`) or as the page of the module (`page`). */
export function Panel({ layout, onClose }: { layout: "side" | "page"; onClose?: () => void }) {
  const t = useT();
  const me = useMe();
  const chat = useBuddyChat();
  // What this tab has added since the last answer from the server; the server's copy wins by id.
  const [added, setAdded] = useState<BuddyMessage[]>([]);
  const [addedProposals, setAddedProposals] = useState<Proposal[]>([]);
  // A card the person has decided stays on screen with its outcome, whichever list it came from.
  const [settled, setSettled] = useState<Record<string, { state: Settled; proposal: Proposal }>>({});
  const [pending, setPending] = useState<string | null>(null);
  const [text, setText] = useState("");
  const [accepted, setAccepted] = useState(false);
  const input = useRef<HTMLTextAreaElement>(null);
  const end = useRef<HTMLDivElement>(null);
  const log = useRef<HTMLDivElement>(null);

  const send = useMutation(async (message: string) => {
    const exchange = await sendBuddyMessage(message, pageContext());
    setAdded((a) => [...a, exchange.user, exchange.assistant]);
    setAddedProposals((a) => [...a, ...exchange.proposals]);
    invalidate(CHAT);
    return exchange;
  });
  const clear = useMutation(async () => {
    await clearBuddyChat();
    setAdded([]);
    setAddedProposals([]);
    setSettled({});
  });

  const stored = chat.data?.messages ?? [];
  const known = new Set(stored.map((m) => m.id));
  const messages = [...stored, ...added.filter((m) => !known.has(m.id))];
  const storedProposals = chat.data?.proposals ?? [];
  // Oldest first, whichever list a card came from.
  const byId = new Map<string, Proposal>();
  for (const p of [...storedProposals, ...addedProposals, ...Object.values(settled).map((x) => x.proposal)]) byId.set(p.id, p);
  const proposals = [...byId.values()].sort((a, b) => Date.parse(a.created_at) - Date.parse(b.created_at));
  const notice = chat.data?.notice ?? null;
  const showNotice = notice !== null && !accepted && !noticeSeen(notice);
  const staff = me.data?.role !== "student";
  const suggestions: MessageKey[] = staff ? ["buddy.suggest.staff1", "buddy.suggest.staff2"] : ["buddy.suggest.student1", "buddy.suggest.student2"];

  useEffect(() => {
    if (layout === "side") input.current?.focus();
  }, [layout]);
  // Follow the conversation: to the end while a message is on its way; but when a card arrives with
  // the answer, stay at its first line, which a long preview would otherwise scroll out of sight.
  const shownProposals = useRef(0);
  const anchor = useRef<"card" | "end">("end");
  useEffect(() => {
    if (proposals.length > shownProposals.current) anchor.current = "card";
    else if (pending !== null) anchor.current = "end";
    shownProposals.current = proposals.length;
    const cards = log.current?.querySelectorAll<HTMLElement>("[data-card]");
    const newest = cards?.[cards.length - 1];
    if (anchor.current === "card" && newest) newest.scrollIntoView({ block: "start" });
    else end.current?.scrollIntoView({ block: "end" });
  }, [messages.length, pending, proposals.length]);

  async function submit(message: string) {
    const clean = message.trim();
    if (!clean || pending !== null) return;
    setText("");
    setPending(clean);
    const done = await send.run(clean);
    setPending(null);
    // The words are not lost when the answer fails.
    if (!done) setText(clean);
    input.current?.focus();
  }

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    void submit(text);
  }

  function onKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      void submit(text);
    }
  }

  const failure = send.error ?? clear.error;
  const failureText = failure && (failure.status === 429 ? t("buddy.limit") : failure.status === 503 ? t("buddy.unavailable") : t("buddy.failed"));

  return (
    <section className={s.panel} data-layout={layout} role={layout === "side" ? "dialog" : undefined} aria-label={t("buddy.title")}>
      <header className={s.head}>
        <Icon name="chat" size={20} />
        <h2 className={s.title}>{t("buddy.title")}</h2>
        <span className={s.grow} />
        <button type="button" className="icon-btn" aria-label={t("buddy.clear")} title={t("buddy.clear")} disabled={messages.length === 0 || clear.pending} onClick={() => void clear.run()}>
          <Icon name="trash" size={18} />
        </button>
        {onClose && (
          <button type="button" className="icon-btn" aria-label={t("buddy.close")} title={t("buddy.close")} onClick={onClose}>
            <Icon name="close" size={20} />
          </button>
        )}
      </header>

      {showNotice && (
        <div className={s.notice} role="note">
          <p>{notice}</p>
          <button
            type="button"
            className="btn btn-quiet"
            onClick={() => {
              rememberNotice(notice);
              setAccepted(true);
            }}
          >
            {t("buddy.noticeOk")}
          </button>
        </div>
      )}

      <div ref={log} className={s.log} role="log" aria-label={t("buddy.log")} aria-live="polite">
        {messages.length === 0 && proposals.length === 0 && pending === null && (
          <div className={s.empty}>
            <p className="muted">{t("buddy.empty")}</p>
            <div className={s.chips}>
              {suggestions.map((key) => (
                <button key={key} type="button" className={s.chip} onClick={() => void submit(t(key))}>
                  {t(key)}
                </button>
              ))}
            </div>
          </div>
        )}
        {messages.map((m) => (
          <p key={m.id} className={s.message} data-role={m.role}>
            <span className="sr-only">{m.role === "user" ? t("buddy.you") : t("buddy.title")}: </span>
            {m.content}
          </p>
        ))}
        {proposals.map((p) => (
          <ProposalCard key={p.id} proposal={p} settled={settled[p.id]?.state} onSettled={(state) => setSettled((all) => ({ ...all, [p.id]: { state, proposal: p } }))} />
        ))}
        {pending !== null && (
          <>
            <p className={s.message} data-role="user">
              <span className="sr-only">{t("buddy.you")}: </span>
              {pending}
            </p>
            <p className={s.message} data-role="assistant" data-quiet role="status">
              {t("buddy.thinking")}
            </p>
          </>
        )}
        <div ref={end} />
      </div>

      {failureText && (
        <p className={s.error} role="alert">
          {failureText}
        </p>
      )}

      <form className={s.composer} onSubmit={onSubmit}>
        <textarea
          ref={input}
          className={`textarea ${s.input}`}
          rows={2}
          maxLength={MAX_TEXT}
          value={text}
          placeholder={t("buddy.placeholder")}
          aria-label={t("buddy.placeholder")}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKeyDown}
        />
        <button type="submit" className="btn btn-primary" disabled={pending !== null || text.trim() === ""}>
          {t("buddy.send")}
        </button>
      </form>
    </section>
  );
}
