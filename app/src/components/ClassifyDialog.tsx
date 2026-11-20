import { useEffect, useState } from "react";
import { api, asError, type ClassificationProgress, type ClassificationReport, type CoreError } from "../bridge";
import type { Text } from "../i18n";
import { CodeNotices } from "./CodeNotices";
import { Button } from "./ui/button";
import { Progress } from "./ui/progress";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from "./ui/dialog";
import { ErrorNotice } from "./common";
import { reasonText } from "../reasons";
import { classificationOutcome, itemReason } from "../lib/classification";

type Props = {
  t: Text;
  onClose: () => void;
  onSaved: (report: ClassificationReport) => void;
} & ({ id: string; all?: never } | { id?: never; all: true });

export function ClassifyDialog({ id, all, t, onClose, onSaved }: Props) {
  const [preview, setPreview] = useState<ClassificationReport>();
  const [report, setReport] = useState<ClassificationReport>();
  const [progress, setProgress] = useState<ClassificationProgress>();
  const [error, setError] = useState<CoreError>();
  const [loading, setLoading] = useState(true);
  const [running, setRunning] = useState(false);
  const [committing, setCommitting] = useState(false);
  const [stopping, setStopping] = useState(false);

  useEffect(() => {
    let active = true;
    void api.previewClassify(id, all).then(value => {
      if (active) setPreview(value);
    }).catch(e => {
      if (active) setError(asError(e));
    }).finally(() => {
      if (active) setLoading(false);
    });
    return () => { active = false; };
  }, [id, all]);

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
            setReport(status.report);
            onSaved(status.report);
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
      await api.startClassify(id, preview.estimated_tokens, all);
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
  const completed = progress?.completed || 0;
  // The reset time Claude reported, already sanitized by core.
  const resets = report?.stopped?.message.match(/\(resets (.*)\)\. /)?.[1];
  return <Dialog open onOpenChange={value => { if (!value) void close(); }}>
    <DialogContent className="classify-dialog" closeLabel={t.close} showCloseButton={!running}>
      <DialogHeader>
        <DialogTitle>{all ? t.classifyAll : t.classifyTitle}</DialogTitle>
        <DialogDescription>{all ? t.classifyAllExplain : t.classifyExplain}</DialogDescription>
      </DialogHeader>
      {all ? <p data-testid="classify-count">{t.skills}: <bdi>{number.format(preview?.selected || 0)}</bdi></p> : <p dir="auto"><strong>{id}</strong></p>}
      {error && <ErrorNotice error={error} t={t} />}
      {loading && <p role="status">{t.loading}</p>}
      {estimate && <dl className="cost-estimate" data-testid="classify-estimate">
        {[[t.agentCalls, preview.estimated_agent_calls], [t.fixedPerCall, estimate.fixed_tokens_per_call], [t.fixedTotal, estimate.fixed_tokens_total], [all ? t.skillTokensAll : t.skillTokens, estimate.skill_tokens_total], [t.totalTokens, estimate.total_input_tokens], [t.retryTokens, estimate.retry_limit_input_tokens]].map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{number.format(Number(value))}</dd></div>)}
        {/* The calibration's source log is a development path; it never reaches the screen. */}
        {estimate.calibration && <div><dt>{t.estimateBasis}</dt><dd data-testid="classify-estimate-basis">{t.estimateBasisValue}</dd></div>}
      </dl>}
      {running && <div className="operation-progress" role="status" data-testid="classify-progress">
        <p>{committing ? t.committing : stopping ? t.classifyStopping : t.classifying}</p>
        {all && <p>{t.classificationCompleted}: <bdi>{number.format(completed)} / {number.format(total)}</bdi>{progress?.current_id && <> · <bdi dir="auto">{progress.current_id}</bdi></>}</p>}
        <Progress value={total ? completed / total * 100 : null} aria-label={t.progress} />
      </div>}
      {report?.stopped && <div role="alert" className="classification-warning" data-testid="classification-stopped" data-reason={report.stopped.reason_code}>
        <strong>{report.stopped.reason_code === "CANCELLED" ? t.classifyCancelledSaved : reasonText(report.stopped.reason_code, t.locale)}</strong>
        {resets && <p><bdi dir="ltr">{resets}</bdi></p>}
        <p data-testid="classification-stopped-counts">{t.classifiedCount}: <bdi>{number.format(report.stopped.classified)}</bdi> · {t.remainingCount}: <bdi>{number.format(report.stopped.remaining.length)}</bdi></p>
      </div>}
      {report && (!report.stopped || report.items.length > 0) && <div role={fallback ? "alert" : "status"} className={fallback ? "classification-warning" : "notice"} data-testid="classification-result">
        <strong>{!fallback ? (report.stopped ? t.classificationCompleted : all ? t.classifyAllReady : t.classifyReady) : !outcome.sharedReason && !report.items.some(item => item.reason_code) ? t.classifyFallback : outcome.allFallback && all ? t.classifyFallbackAll : all ? t.classifyFallbackSome : t.approximate}</strong>
        {outcome.sharedReason && <p data-testid="classification-reason" data-reason={outcome.sharedReason}>{t.fallbackReason}: {reasonText(outcome.sharedReason, t.locale)}</p>}
        {searched.length > 0 && <div data-testid="classification-searched"><p>{t.searchedLocations}:</p><ul>{searched.map(location => <li key={location}><bdi dir="ltr">{location}</bdi></li>)}</ul></div>}
        {!report.dry_run && <p data-testid="classification-agent-calls">{t.actualAgentCalls}: <bdi>{number.format(report.agent_calls)}</bdi></p>}
        <CodeNotices codes={(report.diagnostic_codes || []).filter(code => !(fallback && code === "CLASSIFICATION_KEYWORD_FALLBACK"))} locale={t.locale} />
        {all && <ul className="classification-items" data-testid="classification-items">{report.items.map(item => { const reason = itemReason(item, outcome); return <li key={item.id} data-reason={reason}><bdi dir="auto">{item.id}</bdi><span>{item.proposal_created ? t.proposal : item.classified_by === "keywords" ? (reason ? `${t.approximate}: ${reasonText(reason, t.locale)}` : t.approximate) : t.classificationCompleted}</span></li>; })}</ul>}
      </div>}
      <DialogFooter>
        <Button variant="outline" data-testid="cancel-classify" disabled={loading || (running && (committing || stopping))} onClick={() => void close()}>{report ? t.close : t.cancel}</Button>
        {preview && !report && <Button data-testid="confirm-classify" disabled={loading || running || !preview.selected} onClick={() => void start()}>{t.confirmClassify}</Button>}
      </DialogFooter>
    </DialogContent>
  </Dialog>;
}
