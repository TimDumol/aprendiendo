import { useState, type ReactNode } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import type { Feedback, Finding, RecordingArtifact, Usage } from "@/lib/types";

type FeedbackPanelProps = {
  feedback: Feedback;
  demo: boolean;
  artifact: RecordingArtifact | null;
  model: string | null;
  elapsedMs: number | null;
  usage: Usage | null;
  onPlayMoment: (seconds: number | null) => void;
};

function formatTokenCount(tokens: number | null): string {
  return tokens === null ? "Unavailable" : tokens.toLocaleString();
}

function formatCost(costUsd: number | null): string {
  if (costUsd === null) return "Unavailable";
  return `~$${costUsd.toFixed(4)}`;
}

function categoryLabel(category: Finding["category"]): string {
  if (category === "language") return "Language";
  if (category === "delivery") return "Delivery";
  return "Intelligibility";
}

function validMoment(finding: Finding, artifact: RecordingArtifact | null): boolean {
  if (!artifact || finding.start_seconds === null || finding.end_seconds === null) return false;
  const duration = artifact.durationMs / 1000;
  return (
    Number.isFinite(finding.start_seconds) &&
    Number.isFinite(finding.end_seconds) &&
    finding.start_seconds >= 0 &&
    finding.end_seconds >= finding.start_seconds &&
    finding.end_seconds <= duration
  );
}

function formatMoment(seconds: number): string {
  const wholeSeconds = Math.max(0, Math.floor(seconds));
  return `${Math.floor(wholeSeconds / 60)}:${String(wholeSeconds % 60).padStart(2, "0")}`;
}

function FindingCard({
  finding,
  demo,
  artifact,
  onPlayMoment,
}: {
  finding: Finding;
  demo: boolean;
  artifact: RecordingArtifact | null;
  onPlayMoment: (seconds: number | null) => void;
}) {
  const momentIsAvailable = validMoment(finding, artifact);
  const quoteIsSupported = finding.quote !== null && finding.quote.trim().length > 0;

  return (
    <View style={styles.findingCard}>
      <View style={styles.findingHeader}>
        <Text style={styles.category} selectable>
          {categoryLabel(finding.category)}
        </Text>
      </View>
      <Text style={styles.observation} selectable>
        {finding.observation}
      </Text>
      {quoteIsSupported ? (
        <View style={styles.quoteBox}>
          <Text style={styles.quoteLabel} selectable>
            {demo
              ? "Illustrative quote"
              : "From the generated transcript · verify in the original take"}
          </Text>
          <Text style={styles.quote} selectable>
            “{finding.quote}”
          </Text>
        </View>
      ) : null}
      {finding.suggestion ? (
        <View style={styles.suggestionBox}>
          <Text style={styles.suggestionLabel} selectable>
            Try this
          </Text>
          <Text style={styles.suggestion} selectable>
            {finding.suggestion}
          </Text>
        </View>
      ) : null}
      {demo ? (
        <Text style={styles.demoMomentNote} selectable>
          Demo moment only — it is not linked to your recording.
        </Text>
      ) : artifact ? (
        <Pressable
          accessibilityLabel={
            momentIsAvailable
              ? `Play approximate moment around ${formatMoment(finding.start_seconds ?? 0)}`
              : "Play recording"
          }
          accessibilityRole="button"
          onPress={() => onPlayMoment(momentIsAvailable ? finding.start_seconds : null)}
          style={({ pressed }) => [styles.momentButton, pressed && styles.pressed]}
        >
          <Text style={styles.momentButtonText}>
            {momentIsAvailable
              ? `Approximate moment · play around ${formatMoment(finding.start_seconds ?? 0)}`
              : "Play recording"}
          </Text>
        </Pressable>
      ) : (
        <Text style={styles.unavailableText} selectable>
          Replay is available after you record a take.
        </Text>
      )}
    </View>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <View style={styles.section}>
      <Text style={styles.sectionTitle} selectable>
        {title}
      </Text>
      {children}
    </View>
  );
}

export function FeedbackPanel({
  feedback,
  demo,
  artifact,
  model,
  elapsedMs,
  usage,
  onPlayMoment,
}: FeedbackPanelProps) {
  const [transcriptOpen, setTranscriptOpen] = useState(false);
  const [detailsOpen, setDetailsOpen] = useState(false);

  return (
    <View style={styles.container}>
      {demo ? (
        <View style={styles.demoBanner}>
          <Text style={styles.demoBannerText} selectable>
            Demo feedback — not an assessment of your recording
          </Text>
        </View>
      ) : null}

      <Section title="What came across">
        <Text style={styles.summary} selectable>
          {feedback.summary}
        </Text>
      </Section>

      <Section title="Keep doing this">
        {feedback.strengths.length ? (
          feedback.strengths.slice(0, 2).map((finding, index) => (
            <FindingCard
              key={`strength-${index}`}
              artifact={artifact}
              demo={demo}
              finding={finding}
              onPlayMoment={onPlayMoment}
            />
          ))
        ) : (
          <Text style={styles.emptyText} selectable>
            No specific strength was returned for this take.
          </Text>
        )}
      </Section>

      <Section title="Work on this next">
        {feedback.improvements.length ? (
          feedback.improvements.slice(0, 2).map((finding, index) => (
            <FindingCard
              key={`improvement-${index}`}
              artifact={artifact}
              demo={demo}
              finding={finding}
              onPlayMoment={onPlayMoment}
            />
          ))
        ) : (
          <Text style={styles.emptyText} selectable>
            No specific improvement was returned. Keep the task in mind and listen back once more.
          </Text>
        )}
      </Section>

      <Pressable
        accessibilityRole="button"
        accessibilityState={{ expanded: transcriptOpen }}
        onPress={() => setTranscriptOpen((open) => !open)}
        style={({ pressed }) => [styles.disclosure, pressed && styles.pressed]}
      >
        <Text style={styles.disclosureText} selectable>
          Transcript {transcriptOpen ? "⌃" : "⌄"}
        </Text>
      </Pressable>
      {transcriptOpen ? (
        <View style={styles.transcriptBox}>
          <Text style={styles.transcript} selectable>
            {feedback.transcript || "No usable speech was transcribed."}
          </Text>
        </View>
      ) : null}

      {feedback.limitations.length ? (
        <View style={styles.limitations}>
          <Text style={styles.limitationsTitle} selectable>
            Limitations
          </Text>
          {feedback.limitations.slice(0, 3).map((limitation, index) => (
            <Text key={`limitation-${index}`} style={styles.limitation} selectable>
              • {limitation}
            </Text>
          ))}
        </View>
      ) : null}

      {!demo ? (
        <View style={styles.usageBox}>
          <Text style={styles.usageTitle} selectable>
            API usage estimate
          </Text>
          <View style={styles.usageRow}>
            <View style={styles.usageMetric}>
              <Text style={styles.usageLabel} selectable>
                Input tokens
              </Text>
              <Text style={styles.usageValue} selectable>
                {formatTokenCount(usage?.input_tokens ?? null)}
              </Text>
            </View>
            <View style={styles.usageMetric}>
              <Text style={styles.usageLabel} selectable>
                Output tokens
              </Text>
              <Text style={styles.usageValue} selectable>
                {formatTokenCount(usage?.output_tokens ?? null)}
              </Text>
            </View>
            <View style={styles.usageMetric}>
              <Text style={styles.usageLabel} selectable>
                Approx. cost
              </Text>
              <Text style={styles.usageValue} selectable>
                {formatCost(usage?.estimated_cost_usd ?? null)}
              </Text>
            </View>
          </View>
          <Text style={styles.usageNote} selectable>
            The estimate uses Gemini Flash paid-tier rates ($0.75/M input and $3.75/M output through 2026-12-31); output cost includes reported thinking tokens. It is not an invoice.
          </Text>
        </View>
      ) : null}

      <Pressable
        accessibilityRole="button"
        accessibilityState={{ expanded: detailsOpen }}
        onPress={() => setDetailsOpen((open) => !open)}
        style={({ pressed }) => [styles.detailsDisclosure, pressed && styles.pressed]}
      >
        <Text style={styles.detailsText} selectable>
          {detailsOpen ? "Hide model details" : "Show model details"}
        </Text>
      </Pressable>
      {detailsOpen ? (
        <Text style={styles.detailsBody} selectable>
          {demo
            ? "Fixture mode; no model call was made."
            : `${model ?? "Configured Gemini model"} · server elapsed ${elapsedMs ?? 0} ms`}
        </Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    gap: 18,
  },
  demoBanner: {
    backgroundColor: "#fff2ce",
    borderColor: "#e7c979",
    borderRadius: 12,
    borderWidth: 1,
    padding: 13,
  },
  demoBannerText: {
    color: "#6b4b18",
    fontSize: 14,
    fontWeight: "800",
    lineHeight: 20,
  },
  section: {
    gap: 11,
  },
  sectionTitle: {
    color: "#2d241f",
    fontSize: 19,
    fontWeight: "800",
  },
  summary: {
    color: "#493d35",
    fontSize: 16,
    lineHeight: 24,
  },
  findingCard: {
    backgroundColor: "#fffaf6",
    borderColor: "#e2d5ca",
    borderRadius: 14,
    borderWidth: 1,
    gap: 10,
    padding: 14,
  },
  findingHeader: {
    alignItems: "flex-start",
    flexDirection: "row",
  },
  category: {
    backgroundColor: "#f1e2d6",
    borderRadius: 99,
    color: "#7c422c",
    fontSize: 11,
    fontWeight: "800",
    letterSpacing: 0.4,
    overflow: "hidden",
    paddingHorizontal: 9,
    paddingVertical: 5,
    textTransform: "uppercase",
  },
  observation: {
    color: "#2d241f",
    fontSize: 15,
    fontWeight: "700",
    lineHeight: 22,
  },
  quoteBox: {
    backgroundColor: "#f8f0eb",
    borderLeftColor: "#b95f38",
    borderLeftWidth: 3,
    gap: 4,
    paddingHorizontal: 11,
    paddingVertical: 8,
  },
  quoteLabel: {
    color: "#8f6a58",
    fontSize: 11,
    fontWeight: "800",
    textTransform: "uppercase",
  },
  quote: {
    color: "#493d35",
    fontSize: 14,
    fontStyle: "italic",
    lineHeight: 21,
  },
  suggestionBox: {
    gap: 4,
  },
  suggestionLabel: {
    color: "#8f4325",
    fontSize: 12,
    fontWeight: "800",
    textTransform: "uppercase",
  },
  suggestion: {
    color: "#493d35",
    fontSize: 14,
    lineHeight: 21,
  },
  momentButton: {
    alignItems: "center",
    alignSelf: "flex-start",
    borderColor: "#b95f38",
    borderRadius: 10,
    borderWidth: 1,
    justifyContent: "center",
    minHeight: 44,
    paddingHorizontal: 11,
  },
  momentButtonText: {
    color: "#8f4325",
    fontSize: 13,
    fontWeight: "800",
  },
  demoMomentNote: {
    color: "#8f6a58",
    fontSize: 12,
    lineHeight: 18,
  },
  unavailableText: {
    color: "#75685e",
    fontSize: 13,
    lineHeight: 19,
  },
  emptyText: {
    color: "#75685e",
    fontSize: 14,
    lineHeight: 21,
  },
  disclosure: {
    alignItems: "center",
    alignSelf: "flex-start",
    minHeight: 44,
    paddingVertical: 10,
  },
  disclosureText: {
    color: "#6f3c26",
    fontSize: 15,
    fontWeight: "800",
  },
  transcriptBox: {
    backgroundColor: "#fffaf6",
    borderColor: "#e2d5ca",
    borderRadius: 12,
    borderWidth: 1,
    padding: 14,
  },
  transcript: {
    color: "#493d35",
    fontSize: 15,
    lineHeight: 24,
  },
  limitations: {
    backgroundColor: "#f6efe9",
    borderRadius: 12,
    gap: 5,
    padding: 13,
  },
  limitationsTitle: {
    color: "#6f3c26",
    fontSize: 13,
    fontWeight: "800",
  },
  limitation: {
    color: "#75685e",
    fontSize: 13,
    lineHeight: 19,
  },
  detailsDisclosure: {
    alignSelf: "flex-start",
    minHeight: 44,
    justifyContent: "center",
  },
  detailsText: {
    color: "#75685e",
    fontSize: 12,
    fontWeight: "700",
  },
  detailsBody: {
    color: "#75685e",
    fontSize: 12,
    lineHeight: 18,
  },
  usageBox: {
    backgroundColor: "#f6efe9",
    borderRadius: 12,
    gap: 10,
    padding: 13,
  },
  usageTitle: {
    color: "#6f3c26",
    fontSize: 13,
    fontWeight: "800",
  },
  usageRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 18,
  },
  usageMetric: {
    flexGrow: 1,
    gap: 3,
    minWidth: 90,
  },
  usageLabel: {
    color: "#75685e",
    fontSize: 11,
    fontWeight: "700",
    textTransform: "uppercase",
  },
  usageValue: {
    color: "#2d241f",
    fontSize: 16,
    fontVariant: ["tabular-nums"],
    fontWeight: "800",
  },
  usageNote: {
    color: "#75685e",
    fontSize: 12,
    lineHeight: 18,
  },
  pressed: {
    opacity: 0.72,
  },
});
