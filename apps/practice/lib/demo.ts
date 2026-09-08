import type { Feedback } from "./types";

export const DEMO_FEEDBACK: Feedback = {
  summary:
    "You explained a change of plans and made the result easy to follow. The example cards below are illustrative only.",
  transcript:
    "Pues, el tren se canceló y tuve que buscar otra manera de llegar. Al final tomé un autobús.",
  strengths: [
    {
      category: "language",
      observation: "You connected the event and your response clearly.",
      quote: "tuve que buscar otra manera de llegar",
      suggestion: null,
      start_seconds: 12,
      end_seconds: 22,
    },
  ],
  improvements: [
    {
      category: "language",
      observation: "Make the cause-and-effect link explicit when the sequence changes quickly.",
      quote: "el tren se canceló",
      suggestion: "Try adding a connector such as por eso when it helps the listener.",
      start_seconds: 4,
      end_seconds: 9,
    },
    {
      category: "delivery",
      observation: "A short pause before the final action can give the listener more time to follow the story.",
      quote: null,
      suggestion: "Keep the pause, then continue with the action rather than rushing the ending.",
      start_seconds: null,
      end_seconds: null,
    },
  ],
  limitations: [
    "These quotes and moments are part of the demo fixture; they were not heard from the current recording.",
  ],
};
