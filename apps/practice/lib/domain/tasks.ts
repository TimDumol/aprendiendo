import type { TopicId } from "@/lib/types";

export type PracticeMode = "free-speaking" | "four-three-two" | "describe-photo";

export type PhotoTask = {
  id: string;
  version: number;
  title: string;
  scene: string;
  prompt: string;
  provenance: string;
  imageUri: string | null;
  imageRelativePath?: string | null;
  imageSha256?: string | null;
  imageAsset?: number | null;
  assetSha256?: string | null;
};

export type TaskSnapshot = {
  id: string;
  version: number;
  mode: PracticeMode;
  title: string;
  instructions: string;
  prompt: string;
  topicId: TopicId | null;
  photo: PhotoTask | null;
  photoPracticeStyle: "guided" | "simulation";
  preparationSeconds: number;
  roundDurationsSeconds: number[];
  feedbackRelease: "after-round" | "after-exercise";
  rubricVersion: string;
};

export const FOUR_THREE_TWO_SECONDS = [240, 180, 120] as const;

/**
 * The images are project-generated task assets. They are bundled locally so
 * the exercise remains offline; they are practice visuals, not DELE exam
 * photographs or an official exam simulation.
 */
const PHOTO_TASK_MANIFEST: PhotoTask[] = [
  {
    id: "photo-market-morning",
    version: 1,
    title: "Una mañana en el mercado",
    scene: "A neighborhood market with a stall, baskets, and several people choosing food.",
    prompt:
      "Describe la imagen. Habla del lugar, las personas, lo que llevan y lo que están haciendo.",
    provenance: "Project-generated local task visual; not an official exam photograph.",
    imageUri: null,
  },
  {
    id: "photo-bus-stop",
    version: 1,
    title: "Esperando el autobús",
    scene: "People waiting at a bus stop on a cloudy afternoon, with bags and bicycles nearby.",
    prompt:
      "Describe dónde están las personas, qué hacen y qué detalles ves alrededor.",
    provenance: "Project-generated local task visual; not an official exam photograph.",
    imageUri: null,
  },
  {
    id: "photo-family-kitchen",
    version: 1,
    title: "En la cocina",
    scene: "A family preparing a meal together in a bright kitchen.",
    prompt:
      "Describe la cocina y explica qué hacen las personas y cómo es el ambiente.",
    provenance: "Project-generated local task visual; not an official exam photograph.",
    imageUri: null,
  },
  {
    id: "photo-park-afternoon",
    version: 1,
    title: "Una tarde en el parque",
    scene: "A city park with a bench, trees, a child playing, and a person reading.",
    prompt:
      "Describe el parque, la posición de las personas y las actividades que realizan.",
    provenance: "Project-generated local task visual; not an official exam photograph.",
    imageUri: null,
  },
  {
    id: "photo-train-platform",
    version: 1,
    title: "En la estación",
    scene: "Travelers on a train platform checking a timetable and carrying luggage.",
    prompt:
      "Explica qué ocurre en la estación y describe a las personas y sus objetos.",
    provenance: "Project-generated local task visual; not an official exam photograph.",
    imageUri: null,
  },
  {
    id: "photo-birthday-table",
    version: 1,
    title: "Una celebración",
    scene: "A table prepared for a birthday meal with food, drinks, and wrapped gifts.",
    prompt:
      "Describe la mesa y la celebración. ¿Qué crees que va a pasar después?",
    provenance: "Project-generated local task visual; not an official exam photograph.",
    imageUri: null,
  },
  {
    id: "photo-rainy-street",
    version: 1,
    title: "Una calle con lluvia",
    scene: "Pedestrians with umbrellas walking along a wet street beside small shops.",
    prompt:
      "Describe el tiempo, la calle y cómo se mueven las personas.",
    provenance: "Project-generated local task visual; not an official exam photograph.",
    imageUri: null,
  },
  {
    id: "photo-beach-day",
    version: 1,
    title: "Un día en la playa",
    scene: "Friends at a beach with towels, a ball, and a small boat in the distance.",
    prompt:
      "Describe el paisaje, las personas y lo que hacen durante el día.",
    provenance: "Project-generated local task visual; not an official exam photograph.",
    imageUri: null,
  },
  {
    id: "photo-library-study",
    version: 1,
    title: "Estudiando en la biblioteca",
    scene: "Students reading and writing at tables in a quiet library.",
    prompt:
      "Describe el lugar y explica qué hacen las personas y qué materiales usan.",
    provenance: "Project-generated local task visual; not an official exam photograph.",
    imageUri: null,
  },
  {
    id: "photo-garden-work",
    version: 1,
    title: "Trabajando en el jardín",
    scene: "Two neighbors planting flowers in a shared garden beside a small house.",
    prompt:
      "Describe el jardín y las acciones de las personas. Menciona los detalles importantes.",
    provenance: "Project-generated local task visual; not an official exam photograph.",
    imageUri: null,
  },
  {
    id: "photo-cafe-conversation",
    version: 1,
    title: "Una conversación en un café",
    scene: "Two friends talking at a café table while a server brings drinks.",
    prompt:
      "Describe la conversación, el café y la relación entre las personas.",
    provenance: "Project-generated local task visual; not an official exam photograph.",
    imageUri: null,
  },
  {
    id: "photo-moving-day",
    version: 1,
    title: "Un día de mudanza",
    scene: "People carrying boxes from a home to a small moving van.",
    prompt:
      "Describe la casa, las cajas y lo que están haciendo las personas.",
    provenance: "Project-generated local task visual; not an official exam photograph.",
    imageUri: null,
  },
];

const PHOTO_ASSETS: Record<string, number> = {
  "photo-market-morning": require("../../assets/tasks/market-morning.jpg"),
  "photo-bus-stop": require("../../assets/tasks/bus-stop.jpg"),
  "photo-family-kitchen": require("../../assets/tasks/family-kitchen.jpg"),
  "photo-park-afternoon": require("../../assets/tasks/park-afternoon.jpg"),
  "photo-train-platform": require("../../assets/tasks/train-platform.jpg"),
  "photo-birthday-table": require("../../assets/tasks/birthday-table.jpg"),
  "photo-rainy-street": require("../../assets/tasks/rainy-street.jpg"),
  "photo-beach-day": require("../../assets/tasks/beach-day.jpg"),
  "photo-library-study": require("../../assets/tasks/library-study.jpg"),
  "photo-garden-work": require("../../assets/tasks/garden-work.jpg"),
  "photo-cafe-conversation": require("../../assets/tasks/cafe-conversation.jpg"),
  "photo-moving-day": require("../../assets/tasks/moving-day.jpg"),
};

const PHOTO_ASSET_HASHES: Record<string, string> = {
  "photo-market-morning": "bfa5950bb722aa0976558bc96bb6594463f6816cb5b1883e7b3dfc38cd6180ce",
  "photo-bus-stop": "855ee5a074d009a6290c49b966a75e69cfae5465263f6fb1e056f11e0cf6c50a",
  "photo-family-kitchen": "58fbac79562c2dbd2c5bc4d4c6108cc8c52d056069d863c1ba39a955565aae34",
  "photo-park-afternoon": "4a423e765fee94ba5ecfb124c21fdfd40c3c79ea85a3ed50723c18e82e240b8a",
  "photo-train-platform": "72e353daad293cbb8917e1a18b1c1d48ec873e63925ba9e8ef1763a8e93be6c3",
  "photo-birthday-table": "efb8b29adb52c01c7e3a2aa29025aae33ebd4f9c0d361eff7ea612880ab2afaf",
  "photo-rainy-street": "cb269b767412625e4a19906331746ff1d85609482c91307d83a7e729b5c4fb05",
  "photo-beach-day": "842c77233b471e19a7eda163bad5a874c267e2ddff9ad77243073e99a40fb656",
  "photo-library-study": "80b85b781089457a20a43e057ac6dbb629a1af34d1622bec137b2c2ecfba0075",
  "photo-garden-work": "b14f73e6e77343bbe09f7080c372a380d6e518f0347cdb1086756dc421a1957e",
  "photo-cafe-conversation": "59b179858e58fc750fb4d0f559aadbcfc6ae01bf09aca3107c66a191422ee7b2",
  "photo-moving-day": "dcd2c3966346e04ec999a3c859a0d749a169453b99668e1f4931eb1d8e361d1d",
};

export const PHOTO_TASKS: PhotoTask[] = PHOTO_TASK_MANIFEST.map((task) => ({
  ...task,
  imageAsset: PHOTO_ASSETS[task.id] ?? null,
  assetSha256: PHOTO_ASSET_HASHES[task.id] ?? null,
}));

export const FREE_TOPIC_DECK = [
  "Una rutina que quieres cambiar",
  "Algo que ocurrió recientemente en tu barrio",
  "Un plan para el próximo fin de semana",
  "Cómo resolviste un pequeño problema",
  "Una opinión sobre una actividad cotidiana",
];

export function buildTaskSnapshot(
  mode: PracticeMode,
  options: {
    topicId?: TopicId | null;
    prompt?: string;
    photo?: PhotoTask | null;
    photoPracticeStyle?: "guided" | "simulation";
    preparationSeconds?: number;
    roundDurationsSeconds?: number[];
  } = {},
): TaskSnapshot {
  const photo = options.photo ?? null;
  const topicId = options.topicId ?? null;
  const prompt = options.prompt ?? photo?.prompt ?? "Habla en español sobre algo real que te interese.";
  const roundDurationsSeconds = options.roundDurationsSeconds ??
    (mode === "four-three-two" ? [...FOUR_THREE_TWO_SECONDS] : [120]);

  return {
    id: `${mode}-${photo?.id ?? topicId ?? "free"}-${Date.now()}`,
    version: 1,
    mode,
    title:
      mode === "four-three-two"
        ? "Práctica 4–3–2"
        : mode === "describe-photo"
          ? photo?.title ?? "Describe una imagen"
          : "Habla libremente",
    instructions:
      mode === "four-three-two"
        ? "Cuenta el mismo mensaje en cuatro, tres y dos minutos. Puedes descansar antes de empezar la siguiente ronda."
        : mode === "describe-photo"
          ? "Describe una fotografía cotidiana con tus propias palabras."
          : "Habla durante uno o dos minutos sobre algo real.",
    prompt,
    topicId,
    photo,
    photoPracticeStyle: options.photoPracticeStyle ?? "guided",
    preparationSeconds: options.preparationSeconds ?? (mode === "describe-photo" ? 120 : 0),
    roundDurationsSeconds,
    feedbackRelease: mode === "four-three-two" ? "after-exercise" : "after-round",
    rubricVersion: "mobile-v1",
  };
}
