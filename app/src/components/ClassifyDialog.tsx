import { useEffect, useState } from "react";
import { api, asError, type ClassificationProgress, type ClassificationReport, type CoreError, type SkippedClassification } from "../bridge";
import type { Text } from "../i18n";
import { CodeNotices } from "./CodeNotices";
import { Button } from "./ui/button";
import { Progress } from "./ui/progress";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from "./ui/dialog";
import { Choice, ErrorNotice } from "./common";
import { reasonText } from "../reasons";
import { classificationOutcome, itemReason, showProviderDetails, STOPS_THAT_ASK, unreviewedPiVersion } from "../lib/classification";
import { settingsApi } from "../bridge";
import { settingsMessages } from "../settings-i18n";
import { piNotes } from "./PiSettings";

export type ClassifySelection =
  | { id: string; all?: never; pending?: never; importedIds?: never }
  | { id?: never; all: true; pending?: never; importedIds?: never }
  | { id?: never; all?: never; pending: true; importedIds?: never }
  | { id?: never; all?: never; pending?: never; importedIds: string[] };

type BatchAgent = "claude" | "auto" | "keywords" | "pi" | "codex";

type Props = {
  chooseBatchAgent?: boolean;
  t: Text;
  onClose: () => void;
  onSaved: (report: ClassificationReport) => void;
} & ClassifySelection;

export function ClassifyDialog({ id, all: selectAll, pending, importedIds, chooseBatchAgent, t, onClose, onSaved }: Props) {
  const [keywordId, setKeywordId] = useState<string>();
  const all = !keywordId && (selectAll || pending || !!importedIds);
  const batchChoice = !!importedIds || !!chooseBatchAgent;
  const [preview, setPreview] = useState<ClassificationReport>();
  const [report, setReport] = useState<ClassificationReport>();
  const [skippedResults, setSkippedResults] = useState<SkippedClassification[]>([]);
  const [progress, setProgress] = useState<ClassificationProgress>();
  const [error, setError] = useState<CoreError>();
  const [loading, setLoading] = useState(true);
  const [running, setRunning] = useState(false);
  const [committing, setCommitting] = useState(false);
  const [stopping, setStopping] = useState(false);
  // The user's explicit choice after Codex was refused or `auto` stopped.
  const [agent, setAgent] = useState<BatchAgent>();
  // After a stop: exactly the skills that were not classified.
  const [remaining, setRemaining] = useState<string[]|undefined>(importedIds);
  const [piEnabled, setPiEnabled] = useState(false);
  const [piAccount, setPiAccount] = useState("");
  // r5/r6 (owner, 2026-11-24): the saved choice to approve the descriptions
  // of skills that are not risky after the run, and the state of its undo.
  const [autoApprove, setAutoApprove] = useState(true);
  const [showApproved, setShowApproved] = useState(false);
  const [approvalUndo, setApprovalUndo] = useState<"idle" | "confirming" | "busy" | "done">("idle");
  const [approvalError, setApprovalError] = useState<CoreError>();
  // The Pi confirmation the user accepted after a PI_CONFIRMATION_REQUIRED stop.
  const [piToken, setPiToken] = useState<string>();
  // Bumped to reload the preview after a stale Pi approval.
  const [previewNonce, setPreviewNonce] = useState(0);
  // r4: how long the current skill has run, against core's 120-second limit
  // for one skill (both attempts included). Measured here from the moment the
  // window first sees that skill, so it never runs ahead of core.
  const [current, setCurrent] = useState<{ id: string; since: number }>();
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    let active = true;
    void settingsApi.read().then(s => {
      if (!active) return;
      setPiAccount(s.config?.classification_pi?.account || "");
      setPiEnabled(s.config?.classification_pi?.enabled ?? false);
      setAutoApprove(s.config?.auto_approve_descriptions ?? true);
      if (batchChoice) setAgent((s.config?.classification_agent || "claude") as BatchAgent);
    }).catch(e => { if (active && batchChoice) { setError(asError(e)); setLoading(false); } });
    return () => { active = false; };
  }, [batchChoice]);
  // Skills whose description this run approved automatically, until undone.
  const approvedIds = approvalUndo === "done" ? [] : report?.auto_approved?.ids ?? [];
  const approvedLabel = t.locale === "ar" ? "وصف معتمد تلقائيًا (غير مرتفعة المخاطر)" : "Approved automatically (not high risk)";
  // r6 (owner, 2026-11-24): a provider's proposal equal to the description
  // the user chose creates no proposal. Core reports it as a provider
  // classification without one; every other provider result has one.
  const unchanged = (item: { classified_by: string; proposal_created: boolean }) => !item.proposal_created && item.classified_by !== "keywords";
  const unchangedLabel = t.locale === "ar" ? "اكتمل؛ الوصف الحالي لم يتغير" : "Completed; the current description is unchanged";
  // «اعتُمد N وصفًا تلقائيًا» with the number's own agreement in Arabic.
  const approvedCount = (n: number) => {
    if (t.locale !== "ar") return `${n} description${n === 1 ? "" : "s"} approved automatically`;
    const shown = new Intl.NumberFormat("ar").format(n);
    if (n === 1) return "اعتُمد وصف واحد تلقائيًا";
    if (n === 2) return "اعتُمد وصفان تلقائيًا";
    if (n >= 3 && n <= 10) return `اعتُمدت ${shown} أوصاف تلقائيًا`;
    return `اعتُمد ${shown} وصفًا تلقائيًا`;
  };
  async function undoApproval() {
    const approved = report?.auto_approved;
    if (!approved || !report || approvalUndo !== "confirming") return;
    setApprovalUndo("busy");
    setApprovalError(undefined);
    try {
      await api.undoAutoApproval(approved.commit);
      setApprovalUndo("done");
      setShowApproved(false);
      onSaved(report);
    } catch (e) {
      setApprovalError(asError(e));
      setApprovalUndo("idle");
    }
  }

  useEffect(() => {
    if (batchChoice && !agent) return;
    let active = true;
    setLoading(true);
    void api.previewClassify(id, selectAll, agent, remaining, pending, batchChoice).then(value => {
      if (active) setPreview(value);
    }).catch(e => {
      if (active) setError(asError(e));
    }).finally(() => {
      if (active) setLoading(false);
    });
    return () => { active = false; };
  }, [id, selectAll, pending, agent, remaining, previewNonce, batchChoice]);

  useEffect(() => {
    const id = progress?.current_id;
    if (!running || !id) return;
    setCurrent(previous => previous?.id === id ? previous : { id, since: Date.now() });
  }, [running, progress?.current_id, progress?.completed, progress?.skipped]);
  useEffect(() => {
    if (!running) { setCurrent(undefined); return; }
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [running]);

  useEffect(() => {
    if (!running) return;
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const status = await api.classifyStatus();
        if (!active) return;
        setCommitting(status.committing);
        setProgress(status.progress);
        if (status.done) {
          setRunning(false);
          setStopping(false);
          await api.cancelClassify();
          if (status.error) setError(status.error);
          if (status.report) {
            const finished = status.report;
            const resolved = new Set(finished.items.map(item => item.id));
            setSkippedResults(previous => {
              const entries = new Map(previous.filter(item => !resolved.has(item.id)).map(item => [item.id, item]));
              for (const item of finished.skipped ?? []) entries.set(item.id, item);
              return [...entries.values()];
            });
            setReport(finished);
            onSaved(finished);
          }
        } else timer = setTimeout(poll, 100);
      } catch (e) {
        if (active) { setError(asError(e)); setRunning(false); }
      }
    }
    void poll();
    return () => { active = false; clearTimeout(timer); };
  }, [running, onSaved]);

  // Cancelling a running job stops it before its next skill; completed skills
  // are saved and the dialog stays open to show how far it got.
  async function close() {
    try {
      if (running) {
        if (await api.cancelClassify()) setStopping(true);
      } else if (await api.cancelClassify()) onClose();
    } catch (e) { setError(asError(e)); }
  }
  async function start() {
    if (!preview) return;
    setError(undefined);
    setLoading(true);
    try {
      // The agent the preview used, so a settings change since is refused.
      await api.startClassify(id, preview.estimated_tokens, selectAll, preview.agent ?? agent, remaining, piToken, pending, batchChoice);
      setRunning(true);
    } catch (e) { setError(asError(e)); }
    finally { setLoading(false); }
  }

  const estimate = preview?.estimated_tokens;
  const number = new Intl.NumberFormat(t.locale);
  const outcome = classificationOutcome(report?.items || []);
  const fallback = outcome.fallback;
  // Where claude was looked for; the same for every skill of one run.
  const searched = report?.items.find(item => item.searched_locations?.length)?.searched_locations || [];
  const total = progress?.total || preview?.selected || 0;
  const processed = (progress?.completed ?? 0) + (progress?.skipped ?? 0);
  const skipped = report ? skippedResults : [];
  // The reset time Claude reported, already sanitized by core.
  const resets = report?.stopped?.message.match(/\(resets (.*)\)\. /)?.[1];
  // No silent keyword fallback (0.3): the user picks the next step. A named
  // agent that stopped (Claude from this button too) never becomes keywords.
  const askAgent = error?.code === "AGENT_CODEX_UNSUPPORTED" || error?.code === "AGENT_PI_DISABLED" || error?.code === "PI_CONFIRMATION_CHANGED" || STOPS_THAT_ASK.includes(report?.stopped?.reason_code || "");
  const left = report?.stopped?.remaining;
  function choose(next: BatchAgent, one?: string) {
    if (one) {
      setKeywordId(one);
      setRemaining([one]);
    } else if (left?.length) {
      setRemaining(left);
    }
    setPiToken(undefined);
    setError(undefined);
    setReport(undefined);
    setPreview(undefined);
    setProgress(undefined);
    if(agent===next)setPreviewNonce(n=>n+1);
    setAgent(next);
  }
  // Owner, 2026-11-18: Pi runs under `auto` only after this explicit approval.
  // A stale approval (settings changed since) is dropped and asked again
  // from the current preview (review of 8d7a5a4, L4).
  const piAsk = report?.stopped?.reason_code === "PI_CONFIRMATION_REQUIRED" ? report.pi_confirmation : error?.code === "PI_CONFIRMATION_CHANGED" ? preview?.pi_confirmation : undefined;
  // The approval box then shows the reloaded preview's settings, never the
  // stale ones (re-review of 40c6474, L-b).
  useEffect(() => { if (error?.code === "PI_CONFIRMATION_CHANGED") { setPiToken(undefined); setPreview(undefined); setPreviewNonce(n => n + 1); } }, [error?.code]);
  function continueWithPi() { if (!piAsk) return; if (left?.length) setRemaining(left); setPiToken(piAsk.token); setError(undefined); setReport(undefined); if (left?.length) setPreview(undefined); setProgress(undefined); }
  const ar = t.locale === "ar";
  const clock = (ms: number) => { const seconds = Math.max(0, Math.floor(ms / 1000)); return `${number.format(Math.floor(seconds / 60))}:${number.format(seconds % 60).padStart(2, number.format(0))}`; };
  // Core's reason texts name Claude; Pi's own wording for its codes.
  const piReason = (code: string, arabic: boolean) => ({
    AGENT_EXECUTABLE_NOT_FOUND: ["لم يُعثر على أمر pi موثوق؛ لم يُشغَّل Pi.", "No trusted pi command was found; Pi was not started."],
    AGENT_EXECUTABLE_UNTRUSTED: ["أمر pi موجود لكنه لا يجتاز شروط الثقة؛ لم يُشغَّل Pi.", "A pi command exists but fails the trust rule; Pi was not started."],
    AGENT_NODE_UNTRUSTED: ["Pi يحتاج node موثوقًا ولم يجتز node الموجود شروط الثقة؛ لم يُشغَّل Pi.", "Pi needs a trusted node and the one found failed the trust rule; Pi was not started."],
    AGENT_NODE_NOT_FOUND: ["Pi يحتاج node ولم يُعثر عليه؛ لم يُشغَّل Pi.", "Pi needs node and none was found; Pi was not started."],
    AGENT_TOOLS_NOT_ISOLATED: ["ظهر في سجل Pi حدث أداة أو حدث غير معروف، فأُوقف فورًا ولم يُستخدم شيء مما أعاده.", "Pi's stream showed a tool or unknown event; it was stopped at once and nothing it returned was used."],
    AGENT_NOT_LOGGED_IN: ["Pi غير مسجل الدخول لدى المزوّد، أو انتهى دخوله، فتوقف التصنيف. يستخدم Kinana الدخول الذي يحفظه Pi نفسه: سجّل الدخول من جديد في Pi (شغّل pi ثم /login)، ثم أعد التصنيف.", "Pi is not signed in to its provider, or its sign-in has expired, so classification stopped. Kinana uses the sign-in Pi saves itself: sign in again in Pi (run pi, then /login), then classify again."],
    AGENT_USAGE_LIMIT: ["وصل مزوّد Pi إلى حد الاستخدام في خطتك، فتوقف التصنيف. حُفظ ما اكتمل، وأعد المحاولة للباقي لاحقًا.", "Pi's provider reported a usage limit on your plan, so classification stopped. Completed skills are saved; try the rest later."],
    AGENT_CREDIT_EXHAUSTED: ["رصيد مزوّد Pi أو حصته غير كافيين، فتوقف التصنيف. حُفظ ما اكتمل.", "Pi's provider reported that its credit or quota is exhausted, so classification stopped. Completed skills are saved."],
    AGENT_MODEL_UNAVAILABLE: ["تعذّر على Pi استعمال الموديل المحدد: ليس في قائمة موديلاته، أو لا يتيحه المزوّد لحسابك. اختر موديلًا آخر من إعدادات Pi.", "Pi could not use the chosen model: it is not in its model list, or the provider does not offer it to your account. Choose another model in the Pi settings."],
    AGENT_CONNECTION_FAILED: ["فشل طلب Pi إلى مزوّد الموديل فبدأ إعادة المحاولة؛ أُوقف قبلها ولم يُستخدم شيء مما أعاده.", "Pi's request to the model provider failed and it began retrying; it was stopped first and nothing it returned was used."],
    PI_APPEND_SYSTEM_FILE: ["ملف تعليمات Pi الإضافي (APPEND_SYSTEM.md) في مجلد إعداداته يمنع التصنيف عبر كنانة؛ لم يُشغَّل Pi. غيّر اسمه أو انقله مؤقتًا أثناء التصنيف ثم أعده.", "Pi's extra instructions file (APPEND_SYSTEM.md) in its settings folder blocks classifying through Kinana; Pi was not started. Rename or move it while you classify, then put it back."],
    AGENT_VERSION_UNVERIFIED: ["إصدار Pi المثبت غير مراجَع؛ لم يُرسل له أي نص.", "The installed Pi version is not reviewed; it was sent nothing."],
    AGENT_PACKAGE_UNVERIFIED: ["ملفات حزمة Pi لا تطابق الإصدار المراجَع؛ لم يُشغَّل Pi ولم يُرسل له أي نص.", "Pi's package files do not match the reviewed version; Pi was not started and was sent nothing."],
    INVALID_AGENT_PROPOSAL: ["رد Pi غير صالح بعد محاولتين.", "Pi's answer was invalid after two attempts."],
    AGENT_TIMEOUT: ["تجاوز Pi المهلة المحددة لهذه المهارة.", "Pi exceeded the time limit for this skill."],
    INVALID_PI_SETTINGS: ["إعدادات Pi غير صالحة؛ لم يُشغَّل.", "Pi's settings are invalid; it was not started."],
  } as Record<string, string[]>)[code]?.[arabic ? 0 : 1] ?? code;
  // A sign-in, limit, credit or model stop that came from Pi names Pi, not
  // Claude (owner, 2026-11-19, live test report, item 1d).
  const stopFromPi = !!report?.stopped?.providers?.some(p => p.agent === "pi" && p.reason_code === report?.stopped?.reason_code && ["AGENT_NOT_LOGGED_IN", "AGENT_USAGE_LIMIT", "AGENT_CREDIT_EXHAUSTED", "AGENT_MODEL_UNAVAILABLE"].includes(p.reason_code || ""));
  const providerLabel = (name: string) => name === "claude" ? "Claude" : name === "codex" ? "Codex" : name === "pi" ? "Pi" : name;
  const providerStatus = (status: string) => ({ planned: ar ? "سيُستخدم" : "will be used", failed: ar ? "لم ينجح" : "did not succeed", unsupported: ar ? "غير مدعوم للتصنيف" : "not supported for classification", disabled: ar ? "غير مفعّل" : "not enabled", fallback: ar ? "احتياطي إن لم ينجح Claude (تجريبي)" : "fallback if Claude fails (experimental)" } as Record<string, string>)[status] || status;
  const stoppedProviders = report?.stopped?.providers;
  const piVersion = unreviewedPiVersion(report?.stopped?.message || "", stoppedProviders || []);
  const hasStoppedProviders = !!stoppedProviders?.length;
  const providers = stoppedProviders?.length ? stoppedProviders : preview?.providers || [];
  const providerDetails = showProviderDetails(providers, hasStoppedProviders) && (
    <ul className="classification-providers" data-testid="classification-providers">
      {providers.map(p => <li key={p.agent} data-agent={p.agent} data-status={p.status}>
        <bdi>{providerLabel(p.agent)}</bdi>: {providerStatus(p.status)}
        {p.reason_code && p.status !== "planned" && p.reason_code === "AGENT_VERSION_UNVERIFIED" && piVersion && p.agent === "pi"
          ? <span data-testid="pi-unreviewed-version">{ar ? <> — Pi <bdi>{piVersion.installed}</bdi> قيد المراجعة؛ الإصدار المقبول <bdi>{piVersion.accepted}</bdi>.</> : <> — Pi {piVersion.installed} is pending review. Accepted version: {piVersion.accepted}.</>}</span>
          : p.reason_code && p.status !== "planned" ? ` — ${p.agent === "pi" ? piReason(p.reason_code, ar) : reasonText(p.reason_code, t.locale)}` : ""}
      </li>)}
    </ul>
  );
  return <Dialog open onOpenChange={value => { if (!value) void close(); }}>
    <DialogContent className="classify-dialog" closeLabel={t.close} showCloseButton={!running}>
      <DialogHeader>
        <DialogTitle>{importedIds && all ? (ar ? "تصنيف المهارات المستوردة" : "Classify imported skills") : pending && all ? t.classifyPending : all ? t.classifyAll : t.classifyTitle}</DialogTitle>
        <DialogDescription>{keywordId ? (ar ? "ستُصنّف هذه المهارة وحدها بالكلمات المفتاحية عند التأكيد." : "Confirm to classify this skill using keywords.") : pending ? t.classifyPendingExplain : all ? t.classifyAllExplain : t.classifyExplain}</DialogDescription>
      </DialogHeader>
      {all ? <p data-testid="classify-count">{t.skills}: <bdi>{number.format(preview?.selected || 0)}</bdi></p> : <p dir="auto"><strong>{keywordId ?? id}</strong></p>}
      {batchChoice && !running && !report && !keywordId && <Choice testId="scan-classification-agent"
        label={settingsMessages[ar ? "ar" : "en"].agent} value={agent || ""}
        options={[
          {value:"claude",label:"Claude Code"},
          {value:"auto",label:settingsMessages[ar ? "ar" : "en"].auto},
          {value:"pi",label:ar?"Pi — تجريبي":"Pi — experimental",disabled:!piEnabled},
          {value:"codex",label:settingsMessages[ar ? "ar" : "en"].codex,disabled:true},
          {value:"keywords",label:settingsMessages[ar ? "ar" : "en"].keywords},
        ]} onChange={value=>choose(value as BatchAgent)}/>}
      {error && <ErrorNotice error={error} t={t} />}
      {loading && <p role="status">{t.loading}</p>}
      {estimate && <dl className="cost-estimate" data-testid="classify-estimate">
        {[[t.agentCalls, preview.estimated_agent_calls], [t.fixedPerCall, estimate.fixed_tokens_per_call], [t.fixedTotal, estimate.fixed_tokens_total], [all ? t.skillTokensAll : t.skillTokens, estimate.skill_tokens_total], [t.totalTokens, estimate.total_input_tokens], [t.retryTokens, estimate.retry_limit_input_tokens]].map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{number.format(Number(value))}</dd></div>)}
        {(preview?.agent === "claude" || preview?.agent === "auto") && <div data-testid="classify-claude-model"><dt>{ar ? "موديل Claude" : "Claude model"}</dt><dd><bdi>{estimate.claude?.model || (ar ? "افتراضي Claude Code" : "Claude Code default")}</bdi></dd></div>}
        {estimate.claude?.effort && <div><dt>{ar ? "جهد Claude" : "Claude effort"}</dt><dd><bdi>{estimate.claude.effort}</bdi></dd></div>}
        {/* The calibration's source log is a development path; it never reaches the screen. */}
        {estimate.calibration && <div><dt>{t.estimateBasis}</dt><dd data-testid="classify-estimate-basis">{t.estimateBasisValue}</dd></div>}
      </dl>}
      {!hasStoppedProviders && providerDetails}
      {preview && !report && !keywordId && agent !== "keywords" && autoApprove && <p className="settings-help" data-testid="classify-auto-approve-note">{ar ? "بعد التصنيف تُعتمد أوصاف المهارات غير مرتفعة المخاطر تلقائيًا في commit يمكن التراجع عنه، وتبقى مرتفعة المخاطر بانتظار مراجعتك. يمكن إيقاف ذلك من الإعدادات." : "After the run, the descriptions of skills that are not high risk are approved automatically in a commit you can undo; high-risk skills wait for your review. You can turn this off in Settings."}</p>}
      {providers.some(p => p.agent === "pi" && (p.status === "planned" || p.status === "fallback")) && <ul className="classification-providers" data-testid="pi-notes">
        {preview?.pi_confirmation && <li data-testid="pi-preview-model">{ar ? "Pi إن لزم" : "Pi if needed"}: <bdi dir="ltr">{preview.pi_confirmation.provider}</bdi> · <bdi dir="ltr">{preview.pi_confirmation.model}</bdi>{preview.pi_confirmation.model_default && <> ({ar ? "افتراضي Pi لهذا المزوّد" : "Pi's default for this provider"})</>}</li>}
        <li>{piNotes[ar ? "ar" : "en"][(piAccount || "unspecified") as "subscription" | "api_key" | "unspecified"]}</li>
        <li>{piNotes[ar ? "ar" : "en"].uncalibrated}</li>
        <li>{piNotes[ar ? "ar" : "en"].cache}</li>
      </ul>}
      {piAsk && !running && <div role="alert" className="classification-warning" data-testid="pi-confirmation">
        <strong>{ar ? "هل تسمح بالتصنيف عبر Pi (تجريبي)؟" : "Allow classification with Pi (experimental)?"}</strong>
        <dl className="cost-estimate">
          <div><dt>{ar ? "المزوّد" : "Provider"}</dt><dd><bdi dir="ltr">{piAsk.provider}</bdi></dd></div>
          <div><dt>{ar ? "الموديل" : "Model"}</dt><dd data-testid="pi-confirmation-model" data-default={piAsk.model_default ? "true" : "false"}><bdi dir="ltr">{piAsk.model}</bdi>{piAsk.model_default && <> · {ar ? "افتراضي Pi لهذا المزوّد" : "Pi's default for this provider"}</>}</dd></div>
          <div><dt>{ar ? "أقصى عدد للاستدعاءات" : "Most calls"}</dt><dd data-testid="pi-confirmation-calls">{number.format(piAsk.max_calls)}</dd></div>
        </dl>
        <p>{ar ? `لـ${number.format(piAsk.skills)} مهارة، استدعاء لكل مهارة ومحاولة إضافية واحدة إن لزم. الرفض يوقف التصنيف ولا يكتب شيئًا للمهارات غير المكتملة.` : `For ${piAsk.skills} skills: one call each, plus one retry if needed. Declining stops here and writes nothing for the skills not classified.`}</p>
        <div className="button-row"><Button data-testid="pi-confirm" disabled={loading} onClick={continueWithPi}>{ar ? "وافق وصنّف عبر Pi" : "Approve and classify with Pi"}</Button></div>
      </div>}
      {askAgent && !running && <div className="button-row" data-testid="classify-choose-agent">
        <Button data-testid="classify-with-claude" disabled={loading} onClick={() => choose("claude")}>{piVersion ? (ar ? "استخدم Claude لهذه الدفعة" : "Use Claude for this batch") : left?.length ? (ar ? `صنّف المتبقية (${number.format(left.length)}) عبر Claude` : `Classify the remaining ${left.length} with Claude`) : (ar ? "صنّف عبر Claude" : "Classify with Claude")}</Button>
        <Button variant="outline" data-testid="classify-with-keywords" disabled={loading} onClick={() => choose("keywords")}>{left?.length ? (ar ? `صنّف المتبقية (${number.format(left.length)}) بالكلمات المفتاحية (تقريبي)` : `Classify the remaining ${left.length} with keywords (approximate)`) : (ar ? "صنّف بالكلمات المفتاحية (تقريبي)" : "Classify with keywords (approximate)")}</Button>
      </div>}
      {running && <div className="operation-progress" role="status" data-testid="classify-progress">
        <p>{committing ? t.committing : stopping ? t.classifyStopping : t.classifying}</p>
        {all && <p>{t.progress}: <bdi>{number.format(processed)} / {number.format(total)}</bdi>{progress?.current_id && <> · <bdi dir="auto">{progress.current_id}</bdi></>}</p>}
        {current && !committing && <p data-testid="classify-elapsed" data-seconds={Math.floor((now - current.since) / 1000)}>{ar ? "الوقت في المهارة الحالية" : "Time on the current skill"}: <bdi dir="ltr">{clock(now - current.since)}</bdi> {ar ? "من" : "of"} <bdi dir="ltr">{clock(120000)}</bdi> <span className="settings-help">{ar ? "(بعدها تُتخطّى المهارة بسببها)" : "(then the skill is skipped with its reason)"}</span></p>}
        <Progress value={total ? processed / total * 100 : null} aria-label={t.progress} />
      </div>}
      {report?.stopped && <div role="alert" className="classification-warning" data-testid="classification-stopped" data-reason={report.stopped.reason_code}>
        <strong>{report.stopped.reason_code === "CANCELLED" ? t.classifyCancelledSaved : stopFromPi ? piReason(report.stopped.reason_code, ar) : reasonText(report.stopped.reason_code, t.locale)}</strong>
        {hasStoppedProviders && providerDetails}
        {resets && <p><bdi dir="ltr">{resets}</bdi></p>}
        <p data-testid="classification-stopped-counts">{t.classifiedCount}: <bdi>{number.format(report.stopped.classified)}</bdi> · {t.remainingCount}: <bdi>{number.format(report.stopped.remaining.length)}</bdi></p>
      </div>}
      {skipped.length > 0 && <div role="alert" className="classification-warning" data-testid="classification-skipped">
        <strong>{ar ? "مهارات بقيت دون تصنيف" : "Skills left unclassified"} · <bdi>{number.format(skipped.length)}</bdi></strong>
        <ul className="classification-items skipped-items">
          {skipped.map(item => <li key={item.id} data-reason={item.reason_code}>
            <bdi dir="auto" className="item-name" title={item.id} data-testid="classification-skipped-name">{item.id}</bdi>
            <span className="item-reason" data-testid="classification-skipped-reason">{item.agent === "pi" ? piReason(item.reason_code, ar) : reasonText(item.reason_code, t.locale)}</span>
            <Button variant="outline" size="sm" disabled={running || loading}
              data-testid={`classify-skipped-keywords-${item.id}`}
              aria-label={ar ? `صنّف ${item.id} بالكلمات المفتاحية` : `Classify ${item.id} with keywords`}
              onClick={() => choose("keywords", item.id)}>
              {ar ? "صنّف هذه بالكلمات المفتاحية" : "Classify this skill with keywords"}
            </Button>
          </li>)}
        </ul>
      </div>}
      {report && ((!report.stopped && skipped.length === 0) || report.items.length > 0) && <div role={fallback ? "alert" : "status"} className={fallback ? "classification-warning" : "notice"} data-testid="classification-result">
        <strong>{!fallback ? (report.stopped || skipped.length > 0 || report.auto_approved || !report.items.some(item => item.proposal_created) ? t.classificationCompleted : all ? t.classifyAllReady : t.classifyReady) : !outcome.sharedReason && !report.items.some(item => item.reason_code) ? t.classifyFallback : outcome.allFallback && all ? t.classifyFallbackAll : all ? t.classifyFallbackSome : t.approximate}</strong>
        {outcome.sharedReason && <p data-testid="classification-reason" data-reason={outcome.sharedReason}>{t.fallbackReason}: {reasonText(outcome.sharedReason, t.locale)}</p>}
        {searched.length > 0 && <div data-testid="classification-searched"><p>{t.searchedLocations}:</p><ul>{searched.map(location => <li key={location}><bdi dir="ltr">{location}</bdi></li>)}</ul></div>}
        {!report.dry_run && <p data-testid="classification-agent-calls">{t.actualAgentCalls}: <bdi>{number.format(report.agent_calls)}</bdi></p>}
        <CodeNotices codes={(report.diagnostic_codes || []).filter(code => !(fallback && code === "CLASSIFICATION_KEYWORD_FALLBACK"))} locale={t.locale} />
        {!all && report.items.length === 1 && unchanged(report.items[0]) && <p data-testid="classify-description-unchanged">{ar ? "الوصف المقترح يطابق الوصف الحالي، فلا شيء ينتظر المراجعة." : "The proposed description matches the current one; nothing waits for review."}</p>}
        {all && <ul className="classification-items" data-testid="classification-items">{report.items.map(item => { const reason = itemReason(item, outcome); const state = item.proposal_created ? (approvedIds.includes(item.id) ? "approved" : "proposal") : item.classified_by === "keywords" ? "keywords" : "unchanged"; return <li key={item.id} data-reason={reason} data-state={state} data-id={item.id}><bdi dir="auto" className="item-name" title={item.id}>{item.id}</bdi><span className="item-reason">{state === "approved" ? approvedLabel : state === "proposal" ? t.proposal : state === "keywords" ? (reason ? `${t.approximate}: ${reasonText(reason, t.locale)}` : t.approximate) : unchangedLabel}</span></li>; })}</ul>}
      </div>}
      {report?.auto_approved && <div role="status" className="notice auto-approval" data-testid="auto-approval" data-state={approvalUndo}>
        {approvalUndo === "done"
          ? <p data-testid="auto-approval-undone">{ar ? `تم التراجع عن الاعتماد التلقائي. عادت ${number.format(report.auto_approved.ids.length)} من الأوصاف تنتظر مراجعتك.` : `The automatic approval was undone; ${report.auto_approved.ids.length} description${report.auto_approved.ids.length === 1 ? "" : "s"} wait for your review again.`}</p>
          : <p className="auto-approval-line"><span data-testid="auto-approval-count" data-count={report.auto_approved.ids.length}>{approvedCount(report.auto_approved.ids.length)}</span>
            {" · "}<Button variant="link" size="sm" data-testid="auto-approval-show" aria-expanded={showApproved} onClick={() => setShowApproved(v => !v)}>{ar ? "عرض" : "Show"}</Button>
            {" · "}<Button variant="link" size="sm" data-testid="auto-approval-undo" disabled={approvalUndo !== "idle"} onClick={() => { setApprovalError(undefined); setApprovalUndo("confirming"); }}>{ar ? "تراجع" : "Undo"}</Button></p>}
        {approvalUndo === "confirming" && <div data-testid="auto-approval-confirmation">
          <p>{ar ? `سيعود ${number.format(report.auto_approved.ids.length)} من الأوصاف إلى انتظار المراجعة.` : `Descriptions that will return to review: ${number.format(report.auto_approved.ids.length)}.`}</p>
          <div className="button-row">
            <Button variant="outline" size="sm" data-testid="cancel-auto-approval-undo" onClick={() => setApprovalUndo("idle")}>{t.cancel}</Button>
            <Button size="sm" data-testid="confirm-auto-approval-undo" onClick={() => void undoApproval()}>{ar ? "تأكيد التراجع" : "Confirm undo"}</Button>
          </div>
        </div>}
        {showApproved && approvalUndo !== "done" && <ul className="classification-items" data-testid="auto-approval-list">{report.auto_approved.ids.map(item => <li key={item}><bdi dir="auto" className="item-name" title={item}>{item}</bdi><span className="item-reason">{approvedLabel}</span></li>)}</ul>}
        {approvalError && <ErrorNotice error={approvalError} t={t} />}
      </div>}
      <DialogFooter>
        <Button variant="outline" data-testid="cancel-classify" disabled={(loading && !batchChoice) || (running && (committing || stopping))} onClick={() => void close()}>{report ? t.close : batchChoice&&!running ? (ar?"تخطَّ التصنيف":"Skip classification") : t.cancel}</Button>
        {preview && !report && <Button data-testid="confirm-classify" disabled={loading || running || !preview.selected} onClick={() => void start()}>{t.confirmClassify}</Button>}
      </DialogFooter>
    </DialogContent>
  </Dialog>;
}
