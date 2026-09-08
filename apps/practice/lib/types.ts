export type FindingCategory = "language" | "delivery" | "intelligibility";

export type Finding = {
  category: FindingCategory;
  observation: string;
  quote: string | null;
  suggestion: string | null;
  start_seconds: number | null;
  end_seconds: number | null;
};

export type Feedback = {
  summary: string;
  transcript: string;
  strengths: Finding[];
  improvements: Finding[];
  limitations: string[];
};

export type AssessmentResponse = {
  feedback: Feedback;
  model: string;
  elapsed_ms: number;
  usage: Usage;
};

export type Usage = {
  input_tokens: number | null;
  output_tokens: number | null;
  thought_tokens: number | null;
  estimated_cost_usd: number | null;
};

export type HealthResponse = {
  status: "ok";
  endpoint: string;
  tokenPresent: boolean;
};

export type TopicId =
  | "change-of-plans"
  | "problem-solved"
  | "enjoyable-experience"
  | "routine-change"
  | "daily-opinion"
  | "own-topic";

export type Topic = {
  id: TopicId;
  label: string;
  prompt: string;
};

export type RecordingArtifact = {
  id: string;
  uri: string;
  relativePath?: string;
  blob?: Blob;
  mimeType: string;
  container?: string;
  codec?: string;
  durationMs: number;
  decodedDurationMs?: number | null;
  clientDurationMs?: number;
  bytes: number;
  hash?: string;
  createdOrder?: number;
  interrupted?: boolean;
  mediaAvailability?: "ready" | "evicted" | "missing";
  objectUrl?: string;
  warning?: string;
};

export type AttemptId = "attempt-1" | "attempt-2";

export type Attempt = {
  id: AttemptId;
  audio: RecordingArtifact | null;
  hasPlayed: boolean;
  feedback: Feedback | null;
  model: string | null;
  elapsedMs: number | null;
  usage: Usage | null;
  requestStatus: "idle" | "loading" | "success" | "error";
  requestError: string | null;
  requestStartedAt: number | null;
};

export type PracticeRunStatus = "preparing" | "ready" | "recording" | "finalizing" | "completed" | "ended";

export type PracticeRoundStatus =
  | "preparing"
  | "ready"
  | "recording"
  | "finalizing"
  | "round_saved"
  | "completed"
  | "interrupted";

export type StoredPracticeRun = {
  id: string;
  snapshot: string;
  mode: string;
  createdOrder: number;
  status: PracticeRunStatus;
  assistance: string | null;
  feedbackExposure: string | null;
  updatedAt: string;
};

export type StoredPracticeRound = {
  id: string;
  runId: string;
  sequence: number;
  targetDurationMs: number;
  classification: "baseline" | "repetition" | "coached" | "image-description";
  status: PracticeRoundStatus;
  interrupted: boolean;
  createdOrder: number;
};

export type StoredAnalysis = {
  id: string;
  recordingId: string;
  recordingHash: string;
  processorVersion: string;
  modelVersion: string;
  configJson: string;
  metricsJson: string;
  status: "pending" | "ready" | "partial" | "failed";
  limitationsJson: string;
  createdAt: string;
};

export type PracticeSettings = {
  quotaGb: number;
  explanationLanguage: "en" | "es";
  preparationSeconds: number;
  photoPracticeStyle: "guided" | "simulation";
  analysisConsent: boolean;
  monthlySpendLimitUsd: number;
};

export const DEFAULT_PRACTICE_SETTINGS: PracticeSettings = {
  quotaGb: 1,
  explanationLanguage: "en",
  preparationSeconds: 120,
  photoPracticeStyle: "guided",
  analysisConsent: true,
  monthlySpendLimitUsd: 1,
};
