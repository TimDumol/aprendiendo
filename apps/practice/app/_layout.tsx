import { Link, Stack } from "expo-router";
import { Component, useEffect, type ErrorInfo, type ReactNode } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { errorMessage, installDiagnostics, logError, logInfo } from "@/lib/logging";

type ErrorBoundaryProps = { children: ReactNode };
type ErrorBoundaryState = { error: Error | null };

class AppErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    logError("react.error-boundary", error, { componentStack: info.componentStack });
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <View style={styles.errorPage}>
        <Text style={styles.errorTitle} selectable>Aprendiendo hit an unexpected error</Text>
        <Text style={styles.errorMessage} selectable>{errorMessage(this.state.error)}</Text>
        <Text style={styles.errorHint} selectable>The error was saved in the developer system log.</Text>
        <Link href={"/logs" as any} asChild>
          <Pressable style={styles.logsButton}><Text style={styles.logsButtonText}>Open system logs</Text></Pressable>
        </Link>
      </View>
    );
  }
}

export default function RootLayout() {
  useEffect(() => {
    installDiagnostics();
    logInfo("app", "Aprendiendo application started");
  }, []);

  return (
    <AppErrorBoundary>
      <Stack
        screenOptions={{
          headerShadowVisible: false,
          headerStyle: { backgroundColor: "#f7efe8" },
          headerTitleStyle: { color: "#2d241f", fontWeight: "800" },
        }}
      >
        <Stack.Screen name="index" options={{ title: "Aprendiendo" }} />
        <Stack.Screen name="history" options={{ title: "History" }} />
        <Stack.Screen name="settings" options={{ title: "Settings & storage" }} />
        <Stack.Screen name="logs" options={{ title: "System logs" }} />
      </Stack>
    </AppErrorBoundary>
  );
}

const styles = StyleSheet.create({
  errorPage: { alignItems: "stretch", backgroundColor: "#f7efe8", flex: 1, gap: 14, justifyContent: "center", padding: 24 },
  errorTitle: { color: "#8c302a", fontSize: 24, fontWeight: "900" },
  errorMessage: { color: "#493d35", fontFamily: "monospace", fontSize: 14, lineHeight: 20 },
  errorHint: { color: "#75685e", fontSize: 14, lineHeight: 20 },
  logsButton: { alignItems: "center", alignSelf: "flex-start", backgroundColor: "#a34f2d", borderRadius: 10, justifyContent: "center", minHeight: 46, paddingHorizontal: 14 },
  logsButtonText: { color: "#fffaf6", fontSize: 14, fontWeight: "800" },
});
