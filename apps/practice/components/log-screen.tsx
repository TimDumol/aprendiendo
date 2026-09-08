import { useCallback, useEffect, useState } from "react";
import { Link } from "expo-router";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";

import {
  APP_ANDROID_VERSION_CODE,
  APP_BUILD_DATE,
  APP_BUILD_VARIANT,
  APP_IDENTIFIER,
  APP_VERSION,
} from "@/lib/build-info";
import {
  clearLogs,
  errorMessage,
  listLogs,
  subscribeLogs,
  type AppLogEntry,
} from "@/lib/logging";

function levelStyle(level: AppLogEntry["level"]) {
  if (level === "error") return styles.levelError;
  if (level === "warn") return styles.levelWarn;
  if (level === "debug") return styles.levelDebug;
  return styles.levelInfo;
}

export function LogScreen() {
  const [entries, setEntries] = useState<AppLogEntry[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(() => {
    setBusy(true);
    void listLogs()
      .then((next) => {
        setEntries(next);
        setLoadError(null);
      })
      .catch((error) => setLoadError(errorMessage(error, "The system log could not be read.")))
      .finally(() => setBusy(false));
  }, []);

  useEffect(() => {
    refresh();
    return subscribeLogs(setEntries);
  }, [refresh]);

  const clear = () => {
    setBusy(true);
    void clearLogs()
      .then(() => setLoadError(null))
      .catch((error) => setLoadError(errorMessage(error, "The system log could not be cleared.")))
      .finally(() => setBusy(false));
  };

  return (
    <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.scrollContent} style={styles.scroll}>
      <View style={styles.page}>
        <View style={styles.linksRow}>
          <Link href={"/settings" as any} asChild>
            <Pressable style={styles.linkButton}>
              <Text style={styles.linkText}>‹ Settings</Text>
            </Pressable>
          </Link>
          <Pressable accessibilityRole="button" disabled={busy} onPress={refresh} style={styles.linkButton}>
            <Text style={styles.linkText}>{busy ? "Working…" : "Refresh"}</Text>
          </Pressable>
          <Pressable accessibilityRole="button" disabled={busy || entries.length === 0} onPress={clear} style={styles.linkButton}>
            <Text style={styles.linkText}>Clear logs</Text>
          </Pressable>
        </View>

        <View style={styles.header}>
          <Text style={styles.title} selectable>System logs</Text>
          <Text style={styles.subtitle} selectable>Developer diagnostics from console output, network calls, auth, storage, recording, and uncaught errors. The newest entry is first; the log retains up to 1,000 entries.</Text>
        </View>

        <View style={styles.card}>
          <Text style={styles.sectionTitle} selectable>Build information</Text>
          <Text style={styles.meta} selectable>Version {APP_VERSION}{APP_ANDROID_VERSION_CODE === null ? "" : ` · Android version code ${APP_ANDROID_VERSION_CODE}`}</Text>
          <Text style={styles.meta} selectable>Build date {APP_BUILD_DATE}</Text>
          <Text style={styles.meta} selectable>Variant {APP_BUILD_VARIANT} · {APP_IDENTIFIER}</Text>
        </View>

        {loadError ? <Text accessibilityLiveRegion="polite" style={styles.error} selectable>{loadError}</Text> : null}

        {entries.length === 0 ? (
          <View style={styles.card}><Text style={styles.empty} selectable>No system log entries have been retained.</Text></View>
        ) : (
          <View style={styles.logList}>
            {entries.map((entry) => (
              <View key={entry.id} style={styles.logCard}>
                <View style={styles.logHeader}>
                  <Text style={[styles.level, levelStyle(entry.level)]} selectable>{entry.level.toUpperCase()}</Text>
                  <Text style={styles.timestamp} selectable>{entry.timestamp}</Text>
                </View>
                <Text style={styles.scope} selectable>{entry.scope}</Text>
                <Text style={styles.message} selectable>{entry.message}</Text>
                {entry.details ? <Text style={styles.details} selectable>{entry.details}</Text> : null}
              </View>
            ))}
          </View>
        )}
      </View>
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: { backgroundColor: "#f7efe8", flex: 1 },
  scrollContent: { paddingBottom: 48, paddingHorizontal: 18, paddingTop: 20 },
  page: { alignSelf: "center", gap: 14, maxWidth: 900, width: "100%" },
  linksRow: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  linkButton: { alignItems: "center", borderColor: "#c9b8a8", borderRadius: 10, borderWidth: 1, justifyContent: "center", minHeight: 44, paddingHorizontal: 12 },
  linkText: { color: "#6f3c26", fontSize: 13, fontWeight: "800" },
  header: { gap: 5 },
  title: { color: "#2d241f", fontSize: 30, fontWeight: "900" },
  subtitle: { color: "#75685e", fontSize: 14, lineHeight: 21 },
  card: { backgroundColor: "#fffaf6", borderColor: "#e2d5ca", borderRadius: 16, borderWidth: 1, gap: 7, padding: 16 },
  sectionTitle: { color: "#2d241f", fontSize: 18, fontWeight: "800" },
  meta: { color: "#493d35", fontFamily: "monospace", fontSize: 12, lineHeight: 18 },
  error: { backgroundColor: "#fbe8e5", borderColor: "#c86a5c", borderRadius: 10, borderWidth: 1, color: "#8c302a", fontFamily: "monospace", fontSize: 13, lineHeight: 19, padding: 12 },
  empty: { color: "#75685e", fontSize: 14 },
  logList: { gap: 9 },
  logCard: { backgroundColor: "#fffaf6", borderColor: "#e2d5ca", borderRadius: 12, borderWidth: 1, gap: 5, padding: 12 },
  logHeader: { alignItems: "center", flexDirection: "row", gap: 10, justifyContent: "space-between" },
  level: { fontFamily: "monospace", fontSize: 11, fontWeight: "900" },
  levelInfo: { color: "#356345" },
  levelDebug: { color: "#75685e" },
  levelWarn: { color: "#8a651c" },
  levelError: { color: "#8c302a" },
  timestamp: { color: "#8a7b70", flex: 1, fontFamily: "monospace", fontSize: 10, textAlign: "right" },
  scope: { color: "#a34f2d", fontFamily: "monospace", fontSize: 11, fontWeight: "800" },
  message: { color: "#2d241f", fontFamily: "monospace", fontSize: 13, lineHeight: 19 },
  details: { color: "#6d6259", fontFamily: "monospace", fontSize: 11, lineHeight: 16 },
});
