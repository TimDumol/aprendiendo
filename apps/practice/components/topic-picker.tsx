import { Pressable, StyleSheet, Text, TextInput, View } from "react-native";

import { TOPICS } from "@/lib/topics";
import type { TopicId } from "@/lib/types";

type TopicPickerProps = {
  selectedTopic: TopicId;
  ownTopicText: string;
  disabled: boolean;
  onSelect: (topic: TopicId) => void;
  onOwnTopicTextChange: (value: string) => void;
};

export function TopicPicker({
  selectedTopic,
  ownTopicText,
  disabled,
  onSelect,
  onOwnTopicTextChange,
}: TopicPickerProps) {
  return (
    <View style={styles.container}>
      <View style={styles.headingRow}>
        <Text style={styles.eyebrow} selectable>
          Speak about something real
        </Text>
        {disabled ? (
          <Text style={styles.frozenLabel} selectable>
            Topic frozen for both attempts
          </Text>
        ) : null}
      </View>

      <View style={styles.options}>
        {TOPICS.map((topic) => {
          const selected = topic.id === selectedTopic;
          return (
            <Pressable
              key={topic.id}
              accessibilityRole="radio"
              accessibilityState={{ checked: selected, disabled }}
              accessibilityLabel={`${topic.label}${selected ? ", selected" : ""}`}
              disabled={disabled}
              onPress={() => onSelect(topic.id)}
              style={({ pressed }) => [
                styles.option,
                selected && styles.optionSelected,
                disabled && styles.optionDisabled,
                pressed && !disabled && styles.optionPressed,
              ]}
            >
              <View style={[styles.radio, selected && styles.radioSelected]}>
                {selected ? <View style={styles.radioDot} /> : null}
              </View>
              <Text style={styles.optionLabel} selectable>
                {topic.label}
              </Text>
            </Pressable>
          );
        })}
      </View>

      {selectedTopic === "own-topic" ? (
        <TextInput
          accessibilityLabel="Optional own topic"
          editable={!disabled}
          multiline
          numberOfLines={3}
          onChangeText={onOwnTopicTextChange}
          placeholder="Optional: describe your own topic"
          placeholderTextColor="#87796d"
          style={[styles.ownTopicInput, disabled && styles.optionDisabled]}
          textAlignVertical="top"
          value={ownTopicText}
        />
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    gap: 14,
  },
  headingRow: {
    alignItems: "baseline",
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 10,
    justifyContent: "space-between",
  },
  eyebrow: {
    color: "#a34f2d",
    fontSize: 13,
    fontWeight: "800",
    letterSpacing: 1.2,
    textTransform: "uppercase",
  },
  frozenLabel: {
    color: "#6d6259",
    fontSize: 12,
  },
  options: {
    gap: 8,
  },
  option: {
    alignItems: "center",
    borderColor: "#d9c9ba",
    borderRadius: 14,
    borderWidth: 1,
    flexDirection: "row",
    gap: 12,
    minHeight: 48,
    paddingHorizontal: 14,
    paddingVertical: 10,
  },
  optionSelected: {
    backgroundColor: "#fff1e7",
    borderColor: "#b95f38",
  },
  optionDisabled: {
    opacity: 0.58,
  },
  optionPressed: {
    backgroundColor: "#f6e8dd",
  },
  radio: {
    alignItems: "center",
    borderColor: "#87796d",
    borderRadius: 99,
    borderWidth: 1.5,
    height: 22,
    justifyContent: "center",
    width: 22,
  },
  radioSelected: {
    borderColor: "#a34f2d",
  },
  radioDot: {
    backgroundColor: "#a34f2d",
    borderRadius: 99,
    height: 10,
    width: 10,
  },
  optionLabel: {
    color: "#2d241f",
    flex: 1,
    fontSize: 15,
    fontWeight: "700",
  },
  ownTopicInput: {
    backgroundColor: "#fffaf6",
    borderColor: "#d9c9ba",
    borderRadius: 14,
    borderWidth: 1,
    color: "#2d241f",
    fontSize: 16,
    lineHeight: 22,
    minHeight: 88,
    padding: 14,
  },
});
