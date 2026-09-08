import type { Topic, TopicId } from "./types";

export const TOPICS: Topic[] = [
  {
    id: "change-of-plans",
    label: "A change of plans",
    prompt: "Cuenta algo que cambió tus planes esta semana. ¿Qué pasó y cómo reaccionaste?",
  },
  {
    id: "problem-solved",
    label: "A problem you solved",
    prompt: "Cuenta un problema que resolviste recientemente. ¿Qué intentaste y qué aprendiste?",
  },
  {
    id: "enjoyable-experience",
    label: "A recent enjoyable experience",
    prompt: "Describe una experiencia reciente que disfrutaste. ¿Qué la hizo especial?",
  },
  {
    id: "routine-change",
    label: "A routine to change",
    prompt: "Explica una rutina cotidiana que quieres cambiar. ¿Por qué y qué paso pequeño puedes dar?",
  },
  {
    id: "daily-opinion",
    label: "An everyday opinion",
    prompt: "Da tu opinión sobre una actividad cotidiana y explica una razón y un ejemplo.",
  },
  {
    id: "own-topic",
    label: "My own topic",
    prompt: "Habla en español sobre algo real que te interese. Explica qué pasó, qué piensas o cómo te sentiste.",
  },
];

export function topicById(id: TopicId): Topic {
  return TOPICS.find((topic) => topic.id === id) ?? TOPICS[0];
}
