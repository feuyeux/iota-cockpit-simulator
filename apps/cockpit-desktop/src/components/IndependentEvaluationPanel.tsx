import { useEffect, useMemo, useState } from "react";
import { CheckCircle2, Clock3, Download, RefreshCw, Scale, ShieldAlert, XCircle } from "lucide-react";
import { simulatorClient } from "../simulatorClient";
import type { EvaluationReportRecord, EvaluationVerdict, SimulationModel } from "../types/simulation";
import { exportEvaluationReportAsJSON } from "../utils/export";
import { useI18n } from "../i18n";

function verdictClass(verdict: EvaluationVerdict): string {
  if (verdict === "pass") return "text-emerald-200";
  if (verdict === "fail") return "text-rose-200";
  return "text-amber-200";
}

function VerdictIcon({ verdict }: { verdict: EvaluationVerdict }) {
  if (verdict === "pass") return <CheckCircle2 className="h-4 w-4 text-emerald-300" />;
  if (verdict === "fail") return <XCircle className="h-4 w-4 text-rose-300" />;
  return <ShieldAlert className="h-4 w-4 text-amber-300" />;
}

export function IndependentEvaluationPanel({ model }: { model: SimulationModel }) {
  const { locale } = useI18n();
  const [history, setHistory] = useState<EvaluationReportRecord[]>([]);
  const [selected, setSelected] = useState<EvaluationReportRecord>();
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string>();
  const text = locale === "zh-CN"
    ? {
        title: "独立评测报告", run: "一键独立评测", running: "评测中…", export: "导出报告",
        unavailable: "运行至少提交一个 tick 后可评测。", gate: "发布门禁", passed: "通过", blocked: "阻断",
        deterministic: "确定性规则", evidence: "证据引用", judges: "Judge 状态", noJudges: "未配置（仅确定性评测）",
        agreement: "双 Judge 一致", disagreement: "Judge 不一致", history: "报告历史", noHistory: "暂无历史报告",
        hashes: "溯源哈希", confidence: "置信度", failed: "独立评测失败"
      }
    : {
        title: "Independent evaluation", run: "Run independent evaluation", running: "Evaluating…", export: "Export report",
        unavailable: "Commit at least one tick before evaluation.", gate: "Release gate", passed: "Passed", blocked: "Blocked",
        deterministic: "Deterministic rules", evidence: "Evidence references", judges: "Judge status", noJudges: "Not configured (deterministic only)",
        agreement: "Two Judges agree", disagreement: "Judge disagreement", history: "Report history", noHistory: "No saved reports",
        hashes: "Provenance hashes", confidence: "Confidence", failed: "Independent evaluation failed"
      };

  useEffect(() => {
    let cancelled = false;
    simulatorClient.listEvaluationReports().then((reports) => {
      if (cancelled) return;
      setHistory(reports);
      const current = reports.find((report) => report.runId === model.runId);
      setSelected(current);
      setError(undefined);
    }).catch((reason: unknown) => {
      if (!cancelled) setError(reason instanceof Error ? reason.message : String(reason));
    });
    return () => { cancelled = true; };
  }, [model.runId]);

  const canEvaluate = Boolean(model.runId && model.scenario?.id && model.tick > 0 && !running);
  const visibleHistory = useMemo(() => history.slice(0, 8), [history]);

  async function evaluate() {
    if (!model.runId || !model.scenario?.id || !canEvaluate) return;
    setRunning(true);
    setError(undefined);
    try {
      const record = await simulatorClient.evaluateRun(model.runId, model.scenario.id);
      setSelected(record);
      setHistory((reports) => [record, ...reports.filter((item) => item.id !== record.id)]);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setRunning(false);
    }
  }

  const report = selected?.report;
  return (
    <div className="border-t border-zinc-800 pt-3" data-testid="independent-evaluation">
      <div className="flex items-center gap-2">
        <Scale className="h-4 w-4 text-violet-300" />
        <h3 className="text-xs font-medium text-zinc-100">{text.title}</h3>
        <button
          className="ml-auto border border-violet-700 px-2 py-1 text-[11px] text-violet-100 transition hover:bg-violet-950 disabled:cursor-not-allowed disabled:opacity-40"
          disabled={!canEvaluate}
          onClick={() => void evaluate()}
        >
          <RefreshCw className={`mr-1 inline h-3 w-3 ${running ? "animate-spin" : ""}`} />
          {running ? text.running : text.run}
        </button>
      </div>
      {!canEvaluate && !running && !report ? <p className="mt-2 text-[11px] text-zinc-500">{text.unavailable}</p> : null}
      {error ? <div className="mt-2 border-l-2 border-rose-400 pl-2 text-[11px] text-rose-200"><strong>{text.failed}:</strong> {error}</div> : null}

      {report && selected ? (
        <div className="mt-3 space-y-3 text-[11px]">
          <div className="flex items-center gap-2 text-zinc-500">
            <span className="truncate" title={selected.scenarioId}>{selected.scenarioId}</span>
            <span>·</span>
            <code className="truncate" title={selected.runId}>{selected.runId}</code>
          </div>
          <div className="flex items-center gap-2 border border-zinc-800 bg-zinc-950/50 p-2">
            <VerdictIcon verdict={report.verdict} />
            <span className={`font-semibold uppercase ${verdictClass(report.verdict)}`}>{report.verdict}</span>
            <span className="ml-auto text-zinc-500">{text.gate}</span>
            <span className={report.releaseGatePassed ? "text-emerald-200" : "text-rose-200"}>
              {report.releaseGatePassed ? text.passed : text.blocked}
            </span>
          </div>
          <p className="leading-5 text-zinc-300">{report.explanation}</p>

          <div>
            <div className="mb-1 font-medium text-zinc-300">{text.deterministic}</div>
            <div className="space-y-1">
              {report.deterministicResults.map((rule) => (
                <div key={rule.ruleId} className="flex items-center gap-2 bg-zinc-950/40 px-2 py-1.5">
                  <VerdictIcon verdict={rule.verdict} />
                  <code className="min-w-0 flex-1 truncate text-zinc-300">{rule.ruleId}</code>
                  <span className={verdictClass(rule.verdict)}>{rule.verdict}</span>
                  <span className="text-zinc-600">t{rule.deadlineTick}</span>
                </div>
              ))}
            </div>
          </div>

          <div>
            <div className="mb-1 font-medium text-zinc-300">{text.evidence}</div>
            <div className="max-h-24 space-y-1 overflow-y-auto">
              {report.evidence.map((reference, index) => (
                <div key={`${reference.tick}-${reference.eventId ?? reference.kind}-${index}`} className="flex gap-2 text-zinc-400">
                  <span className="font-mono text-cyan-300">t{reference.tick}</span>
                  <span className="min-w-0 flex-1 truncate" title={reference.eventId}>{reference.kind}</span>
                  {reference.entityId ? <span className="text-zinc-600">{reference.entityId}</span> : null}
                </div>
              ))}
            </div>
          </div>

          <div>
            <div className="mb-1 font-medium text-zinc-300">{text.judges}</div>
            {report.judges.length === 0 ? <div className="text-zinc-500">{text.noJudges}</div> : (
              <div className="space-y-1">
                <div className={report.judgeDisagreement ? "text-rose-200" : "text-emerald-200"}>
                  {report.judgeDisagreement ? text.disagreement : text.agreement}
                </div>
                {report.judges.map((judge) => (
                  <div key={judge.provenance.judgeId} className="grid grid-cols-[1fr_auto_auto] gap-2 bg-zinc-950/40 px-2 py-1.5">
                    <span className="truncate" title={judge.provenance.judgeId}>{judge.provenance.judgeId} · {judge.provenance.model}</span>
                    <span className={verdictClass(judge.verdict)}>{judge.verdict}</span>
                    <span className="text-zinc-500">{text.confidence} {(judge.confidence * 100).toFixed(0)}%</span>
                  </div>
                ))}
              </div>
            )}
          </div>

          <details>
            <summary className="cursor-pointer text-zinc-400">{text.hashes}</summary>
            <div className="mt-1 space-y-1 font-mono text-[9px] text-zinc-600">
              <div title={report.inputHash}>input {report.inputHash}</div>
              <div title={report.rubricHash}>rubric {report.rubricHash}</div>
              <div title={report.schemaHash}>schema {report.schemaHash}</div>
            </div>
          </details>
          <button className="border border-zinc-700 px-2 py-1 text-[11px] text-zinc-300 hover:bg-zinc-800" onClick={() => exportEvaluationReportAsJSON(selected)}>
            <Download className="mr-1 inline h-3 w-3" />{text.export}
          </button>
        </div>
      ) : null}

      <div className="mt-3">
        <div className="mb-1 flex items-center gap-1 font-medium text-zinc-400"><Clock3 className="h-3 w-3" />{text.history}</div>
        {visibleHistory.length === 0 ? <div className="text-[11px] text-zinc-600">{text.noHistory}</div> : (
          <div className="flex gap-1 overflow-x-auto pb-1">
            {visibleHistory.map((record) => (
              <button
                key={record.id}
                className={`shrink-0 border px-2 py-1 text-[10px] ${selected?.id === record.id ? "border-violet-500 text-violet-100" : "border-zinc-800 text-zinc-500"}`}
                title={new Date(record.createdAtMs).toLocaleString()}
                onClick={() => setSelected(record)}
              >
                {record.scenarioId} · <span className={verdictClass(record.report.verdict)}>{record.report.verdict}</span>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
