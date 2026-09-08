import { Link } from "expo-router";
import { Image, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { useState } from "react";
import { useEffect } from "react";

import type { PracticeMode } from "@/lib/domain/tasks";
import { PHOTO_TASKS } from "@/lib/domain/tasks";
import { PhotoPracticeScreen } from "./photo-practice-screen";
import { PracticeScreen } from "./practice-screen";
import { TimedRoundScreen } from "./timed-round-screen";
import { drainOutbox } from "@/lib/sync/outbox";
import { initializeStorage, listRuns, parseSnapshot } from "@/lib/storage/repository";
import { errorMessage, logError } from "@/lib/logging";
import type { StoredPracticeRun } from "@/lib/types";

type PracticeHomeScreenProps = {
  initialMode?: PracticeMode | null;
};

const MODE_CARDS: Array<{
  mode: PracticeMode;
  label: string;
  description: string;
  detail: string;
}> = [
  {
    mode: "free-speaking",
    label: "Free speaking",
    description: "Talk about a real event, plan, problem, or opinion.",
    detail: "1–2 minutes · repeat with a linked take",
  },
  {
    mode: "four-three-two",
    label: "4–3–2 practice",
    description: "Keep the same message while the time gets shorter.",
    detail: "4 minutes · 3 minutes · 2 minutes",
  },
  {
    mode: "describe-photo",
    label: "Describe a photo",
    description: "Prepare and describe an everyday scene in Spanish.",
    detail: "2-minute preparation · 3-minute task simulation",
  },
];

export function PracticeHomeScreen({ initialMode = null }: PracticeHomeScreenProps) {
  const [mode, setMode] = useState<PracticeMode | null>(initialMode);
  const [resumeRun, setResumeRun] = useState<StoredPracticeRun | null>(null);
  const [startupError, setStartupError] = useState<string | null>(null);

  useEffect(() => {
    void (async () => {
      try {
        await initializeStorage();
        try {
          await drainOutbox();
        } catch (error) {
          logError("practice.outbox", error, { operation: "startup_drain" });
          setStartupError(`Queued cloud work could not be checked: ${errorMessage(error)}`);
        }
        const runs = await listRuns();
        setResumeRun(runs.find((run) => !["completed", "ended"].includes(run.status)) ?? null);
      } catch (error) {
        logError("practice.startup", error);
        setStartupError(`Practice storage could not be opened: ${errorMessage(error)}`);
      }
    })();
  }, []);

  if (mode === "free-speaking") {
    return <PracticeScreen onBack={() => setMode(null)} resumeRunId={resumeRun?.mode === "free-speaking" ? resumeRun.id : undefined} />;
  }
  if (mode === "four-three-two") {
    return <TimedRoundScreen onBack={() => setMode(null)} resumeRunId={resumeRun?.mode === "four-three-two" ? resumeRun.id : undefined} />;
  }
  if (mode === "describe-photo") {
    return <PhotoPracticeScreen photo={resumeRun?.mode === "describe-photo" ? parseSnapshot(resumeRun)?.photo ?? PHOTO_TASKS[0] : PHOTO_TASKS[0]} onBack={() => setMode(null)} resumeRunId={resumeRun?.mode === "describe-photo" ? resumeRun.id : undefined} />;
  }

  return (
    <ScrollView
      contentInsetAdjustmentBehavior="automatic"
      contentContainerStyle={styles.scrollContent}
      style={styles.scroll}
    >
      <View style={styles.page}>
        <View style={styles.headerRow}>
          <Image accessibilityLabel="Aprendiendo logo" source={require("../assets/icon.png")} style={styles.logo} />
          <View style={styles.header}>
            <Text style={styles.brand} selectable>
              Aprendiendo
            </Text>
            <Text style={styles.subtitle} selectable>
              Spanish speaking practice that stays useful offline.
            </Text>
          </View>
        </View>

        <View style={styles.notice}>
          <Text style={styles.noticeText} selectable>
            Your recordings are saved on this device. Cloud coaching is optional and waits until a connection is available.
          </Text>
        </View>

        {startupError ? (
          <View style={styles.errorNotice}>
            <Text accessibilityLiveRegion="polite" style={styles.errorNoticeText} selectable>{startupError}</Text>
            <Link href={"/logs" as any} asChild>
              <Pressable style={styles.errorLink}><Text style={styles.errorLinkText}>Open system logs</Text></Pressable>
            </Link>
          </View>
        ) : null}

        {resumeRun ? (
          <Pressable accessibilityRole="button" onPress={() => setMode(resumeRun.mode as PracticeMode)} style={styles.resumeCard}>
            <Text style={styles.resumeTitle} selectable>Resume {parseSnapshot(resumeRun)?.title ?? "saved practice"}</Text>
            <Text style={styles.resumeText} selectable>Continue at the saved boundary. The microphone will not start automatically.</Text>
          </Pressable>
        ) : null}

        <View style={styles.cardList}>
          <Text style={styles.sectionTitle} selectable>
            Start practice
          </Text>
          {MODE_CARDS.map((card) => (
            <Pressable
              key={card.mode}
              accessibilityRole="button"
              onPress={() => setMode(card.mode)}
              style={({ pressed }) => [styles.modeCard, pressed && styles.pressed]}
            >
              <View style={styles.modeCardHeader}>
                <Text style={styles.modeLabel} selectable>
                  {card.label}
                </Text>
                <Text style={styles.chevron} selectable>
                  ›
                </Text>
              </View>
              <Text style={styles.modeDescription} selectable>
                {card.description}
              </Text>
              <Text style={styles.modeDetail} selectable>
                {card.detail}
              </Text>
            </Pressable>
          ))}
        </View>

        <View style={styles.linksRow}>
          <Link href={"/history" as any} asChild>
            <Pressable style={({ pressed }) => [styles.linkButton, pressed && styles.pressed]}>
              <Text style={styles.linkText}>History</Text>
            </Pressable>
          </Link>
          <Link href={"/settings" as any} asChild>
            <Pressable style={({ pressed }) => [styles.linkButton, pressed && styles.pressed]}>
              <Text style={styles.linkText}>Settings & storage</Text>
            </Pressable>
          </Link>
        </View>

        <Text style={styles.footer} selectable>
          No CEFR prediction or global fluency score. Measurements identify detected pauses and their limits.
        </Text>
      </View>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: { backgroundColor: "#f7efe8", flex: 1 },
  scrollContent: { paddingBottom: 48, paddingHorizontal: 18, paddingTop: 24 },
  page: { alignSelf: "center", gap: 20, maxWidth: 760, width: "100%" },
  headerRow: { alignItems: "center", flexDirection: "row", gap: 12 },
  logo: { borderRadius: 16, height: 64, width: 64 },
  header: { gap: 5 },
  brand: { color: "#2d241f", fontSize: 32, fontWeight: "900", letterSpacing: -0.6 },
  subtitle: { color: "#75685e", fontSize: 15, lineHeight: 22 },
  notice: { backgroundColor: "#f1e2d6", borderRadius: 14, padding: 14 },
  noticeText: { color: "#6f3c26", fontSize: 14, lineHeight: 21 },
  errorNotice: { backgroundColor: "#fbe8e5", borderColor: "#c86a5c", borderRadius: 14, borderWidth: 1, gap: 9, padding: 14 },
  errorNoticeText: { color: "#8c302a", fontFamily: "monospace", fontSize: 13, lineHeight: 19 },
  errorLink: { alignSelf: "flex-start", borderColor: "#c86a5c", borderRadius: 9, borderWidth: 1, minHeight: 42, justifyContent: "center", paddingHorizontal: 11 },
  errorLinkText: { color: "#8c302a", fontSize: 13, fontWeight: "800" },
  resumeCard: { backgroundColor: "#fff1e7", borderColor: "#a34f2d", borderRadius: 14, borderWidth: 1, gap: 5, padding: 14 },
  resumeTitle: { color: "#6f3c26", fontSize: 16, fontWeight: "800" },
  resumeText: { color: "#75685e", fontSize: 13, lineHeight: 19 },
  cardList: { gap: 12 },
  sectionTitle: { color: "#2d241f", fontSize: 20, fontWeight: "800", marginBottom: 2 },
  modeCard: {
    backgroundColor: "#fffaf6",
    borderColor: "#e2d5ca",
    borderRadius: 16,
    borderWidth: 1,
    gap: 7,
    padding: 17,
  },
  modeCardHeader: { alignItems: "center", flexDirection: "row", justifyContent: "space-between" },
  modeLabel: { color: "#2d241f", fontSize: 18, fontWeight: "800" },
  chevron: { color: "#a34f2d", fontSize: 28, lineHeight: 28 },
  modeDescription: { color: "#493d35", fontSize: 15, lineHeight: 22 },
  modeDetail: { color: "#8f6a58", fontSize: 13, lineHeight: 19 },
  linksRow: { flexDirection: "row", flexWrap: "wrap", gap: 10 },
  linkButton: {
    alignItems: "center",
    borderColor: "#c9b8a8",
    borderRadius: 11,
    borderWidth: 1,
    justifyContent: "center",
    minHeight: 46,
    paddingHorizontal: 14,
  },
  linkText: { color: "#6f3c26", fontSize: 14, fontWeight: "800" },
  footer: { color: "#75685e", fontSize: 12, lineHeight: 18 },
  pressed: { opacity: 0.72 },
});
