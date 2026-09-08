export type AnalysisStatus = "pending" | "ready" | "partial" | "failed";

export type SpeechInterval = {
  start_seconds: number;
  end_seconds: number;
};

export type DeliveryAnalysis = {
  recording_hash: string;
  processor_version: string;
  model_version: string;
  config: {
    sample_rate_hz: number;
    probability_threshold: number;
    minimum_speech_ms: number;
    minimum_silence_ms: number;
  };
  duration_seconds: number;
  response_span_seconds: number | null;
  speech_active_seconds: number | null;
  internal_pause_count: number | null;
  long_pause_count: number | null;
  pause_time_seconds: number | null;
  pause_burden: number | null;
  pause_frequency_per_minute: number | null;
  typical_pause_seconds: number | null;
  longest_pause_seconds: number | null;
  speech_intervals: SpeechInterval[];
  limitations: string[];
  status: AnalysisStatus;
};

export type MetricComparison = {
  compatible: boolean;
  reason: string | null;
  rows: Array<{
    label: string;
    duration_seconds: number;
    pause_count: number | null;
    long_pause_count: number | null;
    pause_time_seconds: number | null;
    pause_time_per_minute: number | null;
  }>;
};

export function compareDeliveryAnalyses(
  analyses: Array<{ label: string; analysis: DeliveryAnalysis | null }>,
): MetricComparison {
  const ready = analyses.filter((item) => item.analysis?.status === "ready" && item.analysis);
  if (!ready.length) {
    return { compatible: false, reason: "Analysis is pending connection.", rows: [] };
  }

  const first = ready[0].analysis as DeliveryAnalysis;
  const sameConfig = ready.every(({ analysis }) => {
    const candidate = analysis as DeliveryAnalysis;
    return (
      candidate.processor_version === first.processor_version &&
      candidate.model_version === first.model_version &&
      candidate.config.sample_rate_hz === first.config.sample_rate_hz &&
      candidate.config.probability_threshold === first.config.probability_threshold
    );
  });
  if (!sameConfig || ready.length !== analyses.length) {
    return {
      compatible: false,
      reason: "Compare takes only after they have the same processor settings and usable audio.",
      rows: [],
    };
  }

  return {
    compatible: true,
    reason: null,
    rows: ready.map(({ label, analysis }) => {
      const value = analysis as DeliveryAnalysis;
      return {
        label,
        duration_seconds: value.duration_seconds,
        pause_count: value.internal_pause_count,
        long_pause_count: value.long_pause_count,
        pause_time_seconds: value.pause_time_seconds,
        pause_time_per_minute:
          value.pause_time_seconds === null || value.response_span_seconds === null || value.response_span_seconds <= 0
            ? null
            : (value.pause_time_seconds / value.response_span_seconds) * 60,
      };
    }),
  };
}
