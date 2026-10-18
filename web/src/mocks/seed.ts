import type { Document, Item, Module } from "@/lib/api/types";
import type { FormConfig, QuizConfig } from "@/lib/api/gaps";
import { bullet, code, doc, heading, math, numbered, para, quote, table, video } from "./docs";
import { at, auditRow, NOW_ISO, uid, type Db, type DbCourse, type DbItem, type DbMember, type DbSubmission, type DbUser } from "./db";

export const PASSWORD = "correct horse";
export const TEMP_PASSWORD = "temporary password";

const STUDENTS_305 = [
  ["Kateryna", "Shevchuk"], ["Mark", "Levin"], ["Anna", "Bondarenko"], ["Taras", "Melnyk"], ["Yulia", "Savchenko"],
  ["Pavlo", "Kravchuk"], ["Nina", "Polishchuk"], ["Oleh", "Tkachenko"], ["Vira", "Moroz"], ["Roman", "Lysenko"],
  ["Daria", "Shapoval"], ["Bohdan", "Rudenko"], ["Halyna", "Marchenko"], ["Denys", "Boyko"], ["Iryna", "Kovalchuk"],
  ["Stepan", "Oliynyk"], ["Olha", "Zinchenko"], ["Maksym", "Hrytsenko"], ["Tetiana", "Vovk"], ["Artem", "Pavlenko"],
  ["Sasha", "Danylenko"], ["Liza", "Kuzmenko"], ["Nazar", "Sydorenko"], ["Zoya", "Yaremchuk"],
] as const;

/** Builds the whole mock world. Called once when the worker starts. */
export function createSeed(): Db {
  const email = (n: string, s: string) => `${n}.${s}`.toLowerCase().replace(/[^a-z.]/g, "") + "@northfield.edu";

  // ---- people ----
  const users: DbUser[] = [];
  const person = (key: string, name: string, surname: string, role: DbUser["role"], extra: Partial<DbUser> = {}): string => {
    const id = uid(`user:${key}`);
    users.push({
      id, email: email(name, surname), name, surname, role, status: "active", must_change_password: false,
      created_at: at("2026-08-28 10:00"), password: PASSWORD, last_sign_in: at("2026-10-17 09:12"), ...extra,
    });
    return id;
  };
  person("marta", "Marta", "Kovalska", "admin", { last_sign_in: at("2026-10-16 22:58") });
  const elena = person("elena", "Elena", "Voss", "teacher", { last_sign_in: at("2026-10-16 21:04") });
  const oksana = person("oksana", "Oksana", "Bondar", "teacher", { last_sign_in: at("2026-10-16 18:30") });
  const kowalska = person("kowalska", "Marta", "Kowalska", "teacher");
  const melnyk = person("melnyk", "Andrii", "Melnyk", "teacher");
  const lin = person("lin", "Grace", "Lin", "teacher");
  const kravets = person("kravets", "Hanna", "Kravets", "teacher");
  const maya = person("maya", "Maya", "Okafor", "student", { last_sign_in: at("2026-10-17 14:32") });
  const daniel = person("daniel", "Daniel", "Kovalenko", "student");
  const sofia = person("sofia", "Sofia", "Marchetti", "student");
  const ivan = person("ivan", "Ivan", "Petrenko", "student", { last_sign_in: at("2026-10-16 22:10") });
  const lena = person("lena", "Lena", "Hoffmann", "student");
  const omar = person("omar", "Omar", "Haddad", "student", { password: TEMP_PASSWORD, must_change_password: true, last_sign_in: null, created_at: at("2026-10-15 17:48") });
  const aiko = person("aiko", "Aiko", "Tanaka", "student");
  const tomas = person("tomas", "Tomas", "Novak", "student", { status: "disabled", last_sign_in: at("2026-09-30 12:00") });
  const s305: string[] = STUDENTS_305.map(([n, s]) => person(`s305:${n}`, n, s, "student"));

  // ---- terms, groups ----
  const terms = [
    { id: uid("term:a25"), name: "Autumn 2025", starts_on: "2025-09-01", ends_on: "2025-12-23" },
    { id: uid("term:s26"), name: "Spring 2026", starts_on: "2026-02-02", ends_on: "2026-06-12" },
    { id: uid("term:a26"), name: "Autumn 2026", starts_on: "2026-09-01", ends_on: "2026-12-23" },
    { id: uid("term:s27"), name: "Spring 2027", starts_on: "2027-02-01", ends_on: "2027-06-11" },
  ];
  const [tA25, tS26, tA26] = [terms[0]!.id, terms[1]!.id, terms[2]!.id];

  const groups = [
    { id: uid("group:cs2"), name: "CS-2, year 2", created_at: at("2025-08-30 10:00") },
    { id: uid("group:cs1"), name: "CS-1, year 1", created_at: at("2025-08-30 10:01") },
    { id: uid("group:cs3"), name: "CS-3, year 3", created_at: at("2025-08-30 10:02") },
    { id: uid("group:math2"), name: "MATH-2, year 2", created_at: at("2025-08-30 10:03") },
    { id: uid("group:exch"), name: "Exchange, autumn", created_at: at("2026-08-30 10:04") },
    { id: uid("group:ta"), name: "Teaching assistants", created_at: at("2025-08-30 10:05") },
  ];
  const subgroups = [
    { id: uid("sub:cs2a"), group_id: groups[0]!.id, name: "CS-2A" },
    { id: uid("sub:cs2b"), group_id: groups[0]!.id, name: "CS-2B" },
  ];
  const gm = (group: number, sub: string | null, user: string, since: string) => ({ group_id: groups[group]!.id, subgroup_id: sub, user_id: user, since: at(since) });
  const groupMembers = [
    gm(0, subgroups[0]!.id, maya, "2025-09-01 09:00"), gm(0, subgroups[0]!.id, daniel, "2025-09-01 09:00"), gm(0, subgroups[0]!.id, sofia, "2025-09-01 09:00"),
    gm(0, subgroups[1]!.id, ivan, "2025-09-01 09:00"), gm(0, subgroups[1]!.id, lena, "2026-02-12 09:00"), gm(0, subgroups[1]!.id, omar, "2026-10-15 17:48"),
    gm(4, null, aiko, "2026-09-08 09:00"),
  ];

  // ---- courses ----
  const courses: DbCourse[] = [];
  const members: DbMember[] = [];
  const course = (key: string, code: string, title: string, owner: string, term: string | null, extra: Partial<DbCourse> = {}) => {
    const id = uid(`course:${key}`);
    courses.push({
      id, kind: "regular", title, status: "published", owner_id: owner, term_id: term, created_at: at("2026-08-20 10:00"),
      code, description: "", settings_saved_at: null, ...extra,
    });
    members.push({ course_id: id, user_id: owner, role: "teacher", is_owner: true, source: "direct", group_label: null, left_on: null });
    return id;
  };
  const enroll = (courseId: string, user: string, role: "teacher" | "assistant" | "student" = "student", extra: Partial<DbMember> = {}) =>
    members.push({ course_id: courseId, user_id: user, role, is_owner: false, source: "direct", group_label: null, left_on: null, ...extra });

  const cs201 = course("cs201", "CS 201", "Algorithms and Data Structures", elena, tA26, {
    description: "How to measure an algorithm, then sorting, searching and graphs. Weekly lectures with one assignment per section.",
    settings_saved_at: at("2026-10-12 11:00"),
  });
  const cs215 = course("cs215", "CS 215", "Databases", kowalska, tA26);
  const math112 = course("math112", "MATH 112", "Linear Algebra", melnyk, tA26);
  const hum104 = course("hum104", "HUM 104", "Academic Writing", lin, tA26);
  const cs305 = course("cs305", "CS 305", "Graph Algorithms", elena, tA26);
  const cs199 = course("cs199", "CS 199", "Summer Bootcamp", kowalska, tS26, { status: "archived" });
  const past = [
    course("cs110", "CS 110", "Discrete Mathematics", melnyk, tS26, { status: "archived" }),
    course("cs102", "CS 102", "Programming Fundamentals", elena, tS26, { status: "archived" }),
    course("phys101", "PHYS 101", "Physics I", kravets, tS26, { status: "archived" }),
    course("cs101", "CS 101", "Introduction to Computing", elena, tA25, { status: "archived" }),
    course("math101", "MATH 101", "Calculus I", melnyk, tA25, { status: "archived" }),
  ];
  const tplAlgo = course("tpl-algo", "", "Algorithms and Data Structures", elena, null, { kind: "template", template_updated_at: at("2026-09-02 12:00") });
  const tplGraph = course("tpl-graph", "", "Graph Algorithms", elena, null, { kind: "template", template_updated_at: at("2026-08-28 12:00") });

  // CS 201 roster: 3 + 3 via the CS-2A/CS-2B links, Aiko direct, Tomas left on 10 October.
  enroll(cs201, oksana, "assistant");
  const via = (label: string) => ({ source: "group" as const, group_label: label });
  enroll(cs201, maya, "student", via("CS-2A"));
  enroll(cs201, daniel, "student", via("CS-2A"));
  enroll(cs201, sofia, "student", via("CS-2A"));
  enroll(cs201, ivan, "student", via("CS-2B"));
  enroll(cs201, lena, "student", via("CS-2B"));
  enroll(cs201, omar, "student", via("CS-2B"));
  enroll(cs201, aiko, "student");
  enroll(cs201, tomas, "student", { left_on: at("2026-10-10 12:00") });
  // The other courses of Maya.
  for (const c of [cs215, math112, hum104]) enroll(c, maya, "student", via("CS-2A"));
  enroll(cs305, oksana, "teacher");
  for (const s of s305) enroll(cs305, s, "student");
  for (const c of past) enroll(c, maya, "student");
  void cs199;

  const links = [
    { id: uid("link:cs201:a"), course_id: cs201, group_id: groups[0]!.id, subgroup_id: subgroups[0]!.id, created_at: at("2026-09-01 09:00"), linked_on: at("2026-09-01 09:00") },
    { id: uid("link:cs201:b"), course_id: cs201, group_id: groups[0]!.id, subgroup_id: subgroups[1]!.id, created_at: at("2026-09-01 09:01"), linked_on: at("2026-09-01 09:01") },
  ];

  // ---- content ----
  const modules: Module[] = [];
  const items: DbItem[] = [];
  const mod = (courseId: string, key: string, title: string, status: "draft" | "published", position: number) => {
    const id = uid(`module:${key}`);
    modules.push({ id, course_id: courseId, parent_id: null, position, title, status, publish_at: null, created_at: at("2026-08-25 10:00"), updated_at: at("2026-10-01 10:00") });
    return id;
  };
  const counters = new Map<string, number>();
  const pos = (moduleId: string) => {
    const next = (counters.get(moduleId) ?? 0) + 1;
    counters.set(moduleId, next);
    return next;
  };
  const baseItem = (courseId: string, moduleId: string, key: string, title: string, body: Document, status: "draft" | "published"): Item => ({
    id: uid(`item:${key}`), module_id: moduleId, course_id: courseId, position: pos(moduleId), kind: "text", title,
    body: body as unknown as Item["body"], status, publish_at: null, created_at: at("2026-08-26 10:00"), updated_at: at("2026-10-12 10:00"),
  });
  const lecture = (courseId: string, moduleId: string, key: string, title: string, body: Document, status: "draft" | "published" = "published", extra: Partial<DbItem> = {}) => {
    const it: DbItem = { ...baseItem(courseId, moduleId, key, title, body, status), last_published_at: status === "published" ? at("2026-10-12 10:00") : null, unpublished_changes: false, ...extra };
    items.push(it);
    return it.id;
  };
  type AsgOpts = {
    type?: string; points: string; due?: string | null; attempts?: number | null; late?: boolean; config?: Record<string, unknown>;
    status?: "draft" | "published"; classOn?: string; body?: Document; autoSubmit?: boolean;
  };
  const assignment = (courseId: string, moduleId: string, key: string, title: string, o: AsgOpts) => {
    const type = o.type ?? "text";
    const due = type === "no_submission" ? null : (o.due ?? null);
    const it: DbItem = {
      ...baseItem(courseId, moduleId, key, title, o.body ?? doc(para("Instructions are in the course.")), o.status ?? "published"),
      kind: "assignment",
      assignment: {
        type, config: o.config ?? {}, max_points: o.points, is_bonus: false,
        due_at: due ? at(due) : null,
        on_deadline: due ? (o.autoSubmit === false ? "keep_draft" : "auto_submit") : null,
        accept_late: due ? (o.late ?? false) : null,
        late_until: null,
        max_attempts: o.attempts === undefined ? 1 : o.attempts,
      },
      class_on: o.classOn ? at(o.classOn) : null,
      last_published_at: (o.status ?? "published") === "published" ? at("2026-10-01 10:00") : null,
      unpublished_changes: false,
    };
    items.push(it);
    return it.id;
  };

  const trueFalse = (id: string, text: string): QuizConfig["questions"][number]["options"][number] => ({ id, text });
  const quiz = (qs: QuizConfig["questions"], extra: Partial<QuizConfig> = {}): QuizConfig => ({ questions: qs, shuffle_questions: false, show_correct: true, ...extra });
  const sortingQuiz = quiz([
    { id: "q1", type: "single", prompt: "What is the worst-case running time of merge sort?", points: "2.00", shuffle: true,
      options: [trueFalse("a", "O(n)"), trueFalse("b", "O(n log n)"), trueFalse("c", "O(n²)"), trueFalse("d", "O(log n)")], correct: ["b"] },
    { id: "q2", type: "multi", prompt: "Which of these sorting algorithms are stable? Pick every one that is.", points: "3.00", shuffle: true, partial_credit: true,
      options: [trueFalse("a", "Merge sort"), trueFalse("b", "Insertion sort"), trueFalse("c", "Quicksort"), trueFalse("d", "Heapsort")], correct: ["a", "b"] },
    { id: "q3", type: "number", prompt: "How many comparisons does merging [1, 4, 9] and [2, 3, 10] take?", points: "2.00", options: [], answer: "5", tolerance: "0" },
    { id: "q4", type: "long", prompt: "Explain in one or two sentences why merge sort needs extra memory.", points: "3.00", options: [],
      grading_notes: "Full marks if the answer says the merge step writes into a new list of size n." },
  ]);
  const vectorsQuiz = quiz([
    { id: "q1", type: "single", prompt: "What is the dot product of (1, 2) and (3, 4)?", points: "4.00", options: [trueFalse("a", "10"), trueFalse("b", "11"), trueFalse("c", "14")], correct: ["b"] },
    { id: "q2", type: "number", prompt: "What is the length of the vector (3, 4)?", points: "3.00", options: [], answer: "5", tolerance: "0" },
    { id: "q3", type: "multi", prompt: "Which of these are vector operations?", points: "3.00", options: [trueFalse("a", "Addition"), trueFalse("b", "Scaling"), trueFalse("c", "Sorting")], correct: ["a", "b"] },
  ]);
  const matricesQuiz = quiz([
    { id: "q1", type: "number", prompt: "How many entries does a 3 by 4 matrix have?", points: "5.00", options: [], answer: "12", tolerance: "0" },
    { id: "q2", type: "single", prompt: "Matrix multiplication is…", points: "5.00", options: [trueFalse("a", "Commutative"), trueFalse("b", "Associative")], correct: ["b"] },
  ]);
  const pathsQuiz = quiz([
    { id: "q1", type: "single", prompt: "Which algorithm finds shortest paths with non-negative weights?", points: "5.00", options: [trueFalse("a", "Dijkstra"), trueFalse("b", "Kruskal"), trueFalse("c", "Prim")], correct: ["a"] },
    { id: "q2", type: "number", prompt: "How many edges does a tree with 8 nodes have?", points: "5.00", options: [], answer: "7", tolerance: "0" },
  ]);
  const variants = [
    ["Variant 1: spell checker", "Byte Me"], ["Variant 2: maze solver", null], ["Variant 3: library search index", null], ["Variant 4: social graph", "Off by One"],
    ["Variant 5: scheduling with priority queues", null], ["Variant 6: compression with Huffman codes", null],
    ["Variant 7: route planner on a city map", "Merge Conflict"], ["Variant 8: file deduplication", null], ["Variant 9: autocomplete with a trie", null],
  ] as const;
  const projectForm: FormConfig = {
    note: "Pick a team of up to three, choose one variant and send a link to your repository. Each variant goes to one team only, first come first served.",
    fields: [
      { id: "f1", type: "short", label: "Team name", help: "Shown to other teams during the project reviews.", required: true, options: [], has_answers: true,
        rules: { max_length: "40", unique: true } },
      { id: "f2", type: "choice", label: "Project variant", help: "Variants other teams took are hidden.", required: true, has_answers: true,
        options: variants.map(([text, taken], i) => ({ id: `v${i + 1}`, text, taken_by: taken })), rules: { one_team_per_option: true, multiple: false } },
      { id: "f3", type: "people", label: "Team members", help: "Up to 3 people from this course.", required: true, options: [], has_answers: true,
        rules: { team_size: "1 to 3", one_team_per_student: true } },
      { id: "f4", type: "link", label: "Link to your repository", help: "A link starting with https://. Make sure your teacher can open it.", required: true, options: [], has_answers: true,
        rules: { allowed_sites: "github.com, gitlab.com", https_only: true } },
      { id: "f5", type: "long", label: "Notes for the teacher", help: "", required: false, options: [], has_answers: false, rules: { max_length: "1000" } },
    ],
  };

  // CS 201
  const mFound = mod(cs201, "cs201:found", "Foundations", "published", 1);
  const mSort = mod(cs201, "cs201:sort", "Sorting", "published", 2);
  const mGraphs = mod(cs201, "cs201:graphs", "Graphs", "draft", 3);
  const mProject = mod(cs201, "cs201:project", "Course project", "published", 4);
  const lWhy = lecture(cs201, mFound, "why", "Why complexity matters", doc(para("Before you compare two algorithms you need a way to measure them."), heading(2, "Counting steps"), para("We count the basic steps an algorithm takes, as a function of the input size.")), "published", { read_minutes: 6 });
  const lBig = lecture(cs201, mFound, "bigo", "Big-O in practice", doc(para("Big-O keeps the fastest growing term and drops constants."), bullet("O(1) — constant"), bullet("O(log n) — logarithmic"), bullet("O(n) — linear")), "published", { read_minutes: 12 });
  const aPs1 = assignment(cs201, mFound, "ps1", "Problem set 1: counting operations", { points: "20.00", due: "2026-10-09 23:59", attempts: 1, body: doc(para("Count the basic operations in each of the five loops and give your answer as a function of n.")) });
  const aPres0 = assignment(cs201, mFound, "pres0", "Presentation: complexity in the wild", {
    type: "no_submission", points: "15.00", classOn: "2026-10-08 12:00",
    body: doc(para("A five-minute talk on where complexity matters in a product you use every day.")),
  });
  lecture(cs201, mSort, "divide", "Divide and conquer", doc(
    para("Sorting ", { math: "n" }, " items by comparing pairs takes about ", { math: "n \\log n" }, " comparisons in the worst case, and no comparison sort can do better. Merge sort reaches that bound with one idea: split the list in half, sort each half, then merge the two sorted halves."),
    heading(2, "The idea in three steps"),
    numbered({ b: "Divide." }, " Split the list into two halves of almost equal size."),
    numbered({ b: "Conquer." }, " Sort each half the same way, until a part has one item."),
    numbered({ b: "Combine." }, " Merge the two sorted halves into one sorted list."),
    heading(2, "The merge step"),
    para("Two sorted lists combine by repeatedly taking the smaller of their front items. Each step moves one item, so merging ", { math: "n" }, " items takes ", { math: "n" }, " steps."),
    code("python", "def merge(left, right):\n    result = []\n    i = j = 0\n    while i < len(left) and j < len(right):\n        if left[i] <= right[j]:\n            result.append(left[i])\n            i += 1\n        else:\n            result.append(right[j])\n            j += 1\n    # one side is empty, append the rest of the other\n    return result + left[i:] + right[j:]"),
    heading(2, "Running time"),
    para("Halving the list again and again gives about log ", { math: "n" }, " levels, and each level does ", { math: "n" }, " steps of merging in total. That gives the recurrence:"),
    math("T(n) = 2T\\left(\\frac{n}{2}\\right) + n"),
    quote("A sort is ", { b: "stable" }, " when equal items keep their original order. Merge sort is stable if the merge takes from the left half on ties, which is why the code uses ", { code: "<=" }, "."),
    heading(2, "How it compares"),
    table([["Algorithm", "Worst case", "Extra memory", "Stable"], ["Merge sort", "O(n log n)", "O(n)", "Yes"], ["Quicksort", "O(n²)", "O(log n)", "No"], ["Insertion sort", "O(n²)", "O(1)", "Yes"]]),
    heading(2, "See it step by step"),
    video("dQw4w9WgXcQ"),
  ), "published", { read_minutes: 15, unpublished_changes: true });
  const aMerge = assignment(cs201, mSort, "merge", "Merge sort implementation", {
    points: "30.00", due: "2026-10-23 23:59", attempts: 2, late: false,
    body: doc(
      para("Implement merge sort in Python and explain why it runs in ", { math: "O(n \\log n)" }, " time."),
      numbered("Write ", { code: "merge_sort(items)" }, " that returns a new sorted list."),
      numbered("Don't use ", { code: "sorted()" }, " or ", { code: "list.sort()" }, "."),
      numbered("In two or three sentences, explain the recurrence in your own words."),
      para("Put your code in a code block so it keeps its indentation."),
    ),
  });
  assignment(cs201, mSort, "sortquiz", "Sorting quiz", { type: "quiz", points: "10.00", due: "2026-10-30 23:59", attempts: 1, config: sortingQuiz as unknown as Record<string, unknown>,
    body: doc(para("Answers are saved as you go. You can submit once. Choice and number questions are checked automatically; your teacher reads the last one.")) });
  assignment(cs201, mSort, "pres", "Presentation: sorting in practice", {
    type: "no_submission", points: "15.00", classOn: "2026-10-29 12:00",
    body: doc(
      para("In groups of three, give a five-minute talk on where a sorting algorithm is used outside the classroom, for example in a database, a spreadsheet or a phone's contact list."),
      bullet("Name the algorithm and say why it fits that job."), bullet("Show one example with real numbers."), bullet("Leave a minute for questions."),
      para("You can find your group on the People tab. Slides are optional."),
    ),
  });
  const lQuick = lecture(cs201, mSort, "quick", "Quicksort", doc(para("Quicksort picks a pivot and partitions the list around it.")), "draft", { read_minutes: 9 });
  lecture(cs201, mGraphs, "graph1", "What a graph is", doc(para("A graph is a set of nodes and a set of edges.")), "draft", { read_minutes: 10 });
  lecture(cs201, mGraphs, "graph2", "Storing graphs", doc(para("Adjacency lists and adjacency matrices.")), "draft", { read_minutes: 14 });
  const aProject = assignment(cs201, mProject, "project", "Course project: team and repository", { type: "form", points: "0.00", due: "2026-10-25 23:59", attempts: null, config: projectForm as unknown as Record<string, unknown>,
    body: doc(para("Dr. Voss uses these details to set up the project reviews. Fill in every required field, then submit.")) });
  void lQuick; void lWhy; void lBig;

  // CS 215
  const m215 = mod(cs215, "cs215:labs", "Labs", "published", 1);
  const aLab1 = assignment(cs215, m215, "lab1", "Lab 1: SQL basics", { points: "30.00", due: "2026-09-24 23:59" });
  const aLab2 = assignment(cs215, m215, "lab2", "Lab 2: joins", { points: "30.00", due: "2026-10-08 12:00" });
  const aLab3 = assignment(cs215, m215, "lab3", "Lab 3: indexes and query plans", { points: "30.00", due: "2026-10-20 12:00", attempts: 2 });
  lecture(cs215, m215, "joins", "Joins explained", doc(para("A join combines rows from two tables."), code("sql", "SELECT * FROM a JOIN b ON a.id = b.a_id;")), "published", { read_minutes: 8 });
  // MATH 112
  const mM = mod(math112, "m112:quizzes", "Vectors and matrices", "published", 1);
  const aQ1 = assignment(math112, mM, "q1", "Quiz 1: vectors", { type: "quiz", points: "10.00", due: "2026-10-06 10:00", config: vectorsQuiz as unknown as Record<string, unknown> });
  const aQ2 = assignment(math112, mM, "q2", "Quiz 2: matrices", { type: "quiz", points: "10.00", due: "2026-10-27 10:00", config: matricesQuiz as unknown as Record<string, unknown> });
  lecture(math112, mM, "vectors", "What is a vector", doc(para("A vector has a length and a direction.")), "published", { read_minutes: 7 });
  // HUM 104
  const mH = mod(hum104, "h104:writing", "Writing", "published", 1);
  const aRr0 = assignment(hum104, mH, "rr0", "Reading response 0: introductions", { points: "5.00", due: "2026-09-25 23:59" });
  const aRr1 = assignment(hum104, mH, "rr1", "Reading response 1", { points: "10.00", due: "2026-10-02 23:59", attempts: 2 });
  const aRr2 = assignment(hum104, mH, "rr2", "Reading response 2", { points: "10.00", due: "2026-11-06 23:59", attempts: 2 });
  const aEssay = assignment(hum104, mH, "essay", "Essay outline", { points: "20.00", due: "2026-10-19 09:00" });
  lecture(hum104, mH, "thesis", "Writing a thesis", doc(para("A thesis states what you will argue.")), "published", { read_minutes: 5 });
  // CS 305
  const m305 = mod(cs305, "cs305:main", "Shortest paths", "published", 1);
  const aBfs = assignment(cs305, m305, "bfs", "Lab 2: breadth-first search", { points: "20.00", due: "2026-10-15 23:59", attempts: 1 });
  const aPaths = assignment(cs305, m305, "paths", "Shortest paths quiz", { type: "quiz", points: "10.00", due: "2026-10-21 23:59", config: pathsQuiz as unknown as Record<string, unknown> });
  const aDijk = assignment(cs305, m305, "dijkstra", "Lab 3: Dijkstra", { points: "30.00", due: "2026-10-26 12:00", status: "draft" });
  lecture(cs305, m305, "bfs-lec", "Breadth-first search", doc(para("Visit the nearest nodes first.")), "published", { read_minutes: 11 });
  // Templates
  const mT = mod(tplAlgo, "tpl:algo", "Sorting", "draft", 1);
  lecture(tplAlgo, mT, "tpl-l1", "Divide and conquer", doc(para("Template lecture.")), "draft", { read_minutes: 15 });
  assignment(tplAlgo, mT, "tpl-a1", "Merge sort implementation", { points: "30.00", status: "draft" });
  void tplGraph;

  // ---- submissions and grades ----
  const submissions: DbSubmission[] = [];
  const subFor = (assignmentId: string, student: string) => {
    let s = submissions.find((x) => x.assignment_id === assignmentId && x.student_id === student);
    if (!s) {
      s = { assignment_id: assignmentId, student_id: student, draft: null, attempts: [] };
      submissions.push(s);
    }
    return s;
  };
  const grades: Db["grades"] = [];
  const gradeFor = (assignmentId: string, student: string) => {
    let g = grades.find((x) => x.assignment_id === assignmentId && x.student_id === student);
    if (!g) {
      g = { assignment_id: assignmentId, student_id: student, draft_score: null, draft_feedback: null, published_score: null, published_feedback: null, published_at: null, history: [] };
      grades.push(g);
    }
    return g;
  };
  let attemptNo = 0;
  const submit = (assignmentId: string, student: string, when: string, content: unknown, attempt = 1) => {
    const s = subFor(assignmentId, student);
    s.attempts.unshift({ id: uid(`attempt:${assignmentId}:${student}:${attempt}:${++attemptNo}`), attempt, content, submitted_at: at(when), is_late: false });
  };
  const draft = (assignmentId: string, student: string, when: string, content: unknown) => {
    subFor(assignmentId, student).draft = { content, updated_at: at(when) };
  };
  const publish = (assignmentId: string, student: string, score: string | null, when: string, feedback: string | null = null) => {
    const g = gradeFor(assignmentId, student);
    g.draft_score = score; g.draft_feedback = feedback; g.published_score = score; g.published_feedback = feedback; g.published_at = at(when);
  };
  const setDraftGrade = (assignmentId: string, student: string, score: string, history: { score: string; at: string; auto?: boolean }[]) => {
    const g = gradeFor(assignmentId, student);
    g.draft_score = score;
    g.history = history.map((h) => ({ action: "save_draft" as const, score: h.score, feedback: null, actor_id: h.auto ? null : elena, at: h.at }));
  };
  const answerDoc = (text: string, source?: string): Document =>
    doc(para(text), ...(source ? [code("python", source)] : []));
  const MERGE_CODE = "def merge_sort(items):\n    if len(items) <= 1:\n        return items[:]\n    mid = len(items) // 2\n    return merge(merge_sort(items[:mid]), merge_sort(items[mid:]))";

  // CS 201: problem set 1, published for the whole roster.
  const ps1: [string, string][] = [[maya, "18.50"], [daniel, "15.00"], [sofia, "19.75"], [ivan, "12.00"], [lena, "17.50"], [omar, "20.00"], [aiko, "16.25"], [tomas, "14.00"]];
  for (const [s, score] of ps1) {
    submit(aPs1, s, "2026-10-08 20:00", answerDoc("My counts for the five loops."));
    publish(aPs1, s, score, "2026-10-12 11:00", s === maya ? "Careful counting in questions 1 to 3. In question 4 the inner loop runs n minus i times, not n." : null);
  }
  // Merge sort: Maya has a draft, five students submitted, four have draft grades, Ivan waits.
  draft(aMerge, maya, "2026-10-17 09:41", answerDoc("My merge_sort splits the list in half until each part has one item, then merges the parts back in order.", MERGE_CODE));
  submit(aMerge, daniel, "2026-10-15 18:40", answerDoc("Recursive merge sort.", MERGE_CODE));
  submit(aMerge, sofia, "2026-10-15 20:10", answerDoc("Recursive merge sort.", MERGE_CODE));
  submit(aMerge, ivan, "2026-10-16 21:12", answerDoc("Recursive merge sort.", MERGE_CODE));
  submit(aMerge, omar, "2026-10-16 09:30", answerDoc("Recursive merge sort.", MERGE_CODE));
  submit(aMerge, aiko, "2026-10-14 12:00", answerDoc("Recursive merge sort.", MERGE_CODE));
  setDraftGrade(aMerge, daniel, "22.50", [{ score: "22.50", at: at("2026-10-17 14:02") }, { score: "20.00", at: at("2026-10-16 19:30") }]);
  setDraftGrade(aMerge, sofia, "29.00", [{ score: "29.00", at: at("2026-10-16 17:00") }]);
  setDraftGrade(aMerge, omar, "27.50", [{ score: "27.50", at: at("2026-10-16 18:00") }]);
  setDraftGrade(aMerge, aiko, "28.00", [{ score: "28.00", at: at("2026-10-16 18:20") }]);
  // Presentations
  publish(aPres0, maya, "13.00", "2026-10-09 18:00", "A clear example with the contact list. You ran two minutes over, so leave out the second example next time.");
  // Form: Maya's draft, two other teams submitted.
  draft(aProject, maya, "2026-10-16 14:32", { answers: { f1: "Merge Conflict", f2: "v7", f3: [maya, daniel, sofia], f4: "https://github.com/merge-conflict/route-planner", f5: "" } });
  submit(aProject, ivan, "2026-10-16 22:10", { answers: { f1: "Byte Me", f2: "v1", f3: [ivan], f4: "https://github.com/ipetrenko/spellcheck", f5: "" } });
  submit(aProject, aiko, "2026-10-16 18:45", { answers: { f1: "Off by One", f2: "v4", f3: [aiko, lena], f4: "https://gitlab.com/offbyone/social-graph", f5: "" } });
  // CS 215
  submit(aLab1, maya, "2026-09-24 20:00", answerDoc("SELECT statements.")); publish(aLab1, maya, "28.00", "2026-09-28 10:00");
  submit(aLab2, maya, "2026-10-08 10:00", answerDoc("Joins.")); publish(aLab2, maya, "27.00", "2026-10-08 18:00");
  draft(aLab3, maya, "2026-10-16 22:00", answerDoc("Indexes speed up lookups."));
  // MATH 112
  submit(aQ1, maya, "2026-10-05 21:00", { answers: { q1: "b", q2: "5", q3: ["a", "c"] } });
  publish(aQ1, maya, "8.00", "2026-10-09 12:00");
  void aQ2;
  // HUM 104
  submit(aRr1, maya, "2026-10-02 18:00", answerDoc("My response to the reading."));
  submit(aRr2, maya, "2026-10-17 11:20", answerDoc("My second response."));
  void aRr0; void aEssay;
  // CS 305
  submit(aBfs, s305[0]!, "2026-10-15 20:05", answerDoc("BFS with a queue."));
  submit(aBfs, s305[1]!, "2026-10-15 23:41", answerDoc("BFS with a deque."));
  s305.slice(0, 12).forEach((s, i) => {
    submit(aPaths, s, `2026-10-16 1${i % 10}:00`, { answers: { q1: "a", q2: "7" } });
    setDraftGrade(aPaths, s, "10.00", [{ score: "10.00", auto: true, at: at("2026-10-16 23:00") }]);
  });
  void aDijk;

  const quizResults: Db["quizResults"] = {
    [aQ1]: { q1: { got: "4.00", feedback: null }, q2: { got: "3.00", feedback: null }, q3: { got: "1.00", feedback: "Sorting is not a vector operation, but scaling is." } },
  };

  const sessions = [
    { id: uid("sess:web"), kind: "web" as const, name: "Web browser", created_at: at("2026-10-01 09:00"), last_seen_at: NOW_ISO, expires_at: at("2026-11-17 09:00"), current: true },
    { id: uid("sess:app"), kind: "app" as const, name: "Mobile app", created_at: at("2026-10-03 09:00"), last_seen_at: at("2026-10-17 13:10"), expires_at: at("2027-01-01 09:00"), current: false },
    { id: uid("sess:cli"), kind: "cli" as const, name: "Command line", created_at: at("2026-10-12 09:00"), last_seen_at: at("2026-10-12 09:05"), expires_at: at("2027-01-10 09:00"), current: false },
  ];

  const marta = { id: uid("marta"), name: "Marta", surname: "Kovalska" };
  const voss = { id: elena, name: "Elena", surname: "Voss" };
  const audit = [
    auditRow("au1", at("2026-10-17 21:40"), voss, "course.group.link", { type: "course", id: cs305, label: "CS 305 Graph Algorithms" }),
    auditRow("au2", at("2026-10-17 16:05"), marta, "user.password_reset", { type: "user", id: omar, label: "Omar Haddad" }),
    auditRow("au3", at("2026-10-17 11:20"), voss, "course.member.put", { type: "course", id: cs305, label: "CS 305 Graph Algorithms" }, { user_id: oksana, role: "teacher" }),
    auditRow("au4", at("2026-10-15 17:48"), marta, "user.create", { type: "user", id: omar, label: "Omar Haddad" }, { role: "student" }),
    auditRow("au5", at("2026-10-15 10:05"), marta, "user.disable", { type: "user", id: uid("marchuk"), label: "Oleh Marchuk" }),
    auditRow("au6", at("2026-10-15 10:02"), marta, "course.owner.transfer", { type: "course", id: uid("phys110"), label: "PHYS 110 Physics" }),
  ];

  const institution = {
    name: "Northfield University", short_name: "Northfield", language: "en" as const, time_zone: "Europe/Kyiv", grading_scale: "hundred" as const,
    mode: "university" as const, allowed_domains: ["northfield.edu"],
  };

  return {
    users, terms, groups, subgroups, groupMembers, courses, members, links, modules, items, submissions, grades, sessions, audit, institution,
    mail: [], signups: [], resets: [], formResponses: {}, quizResults,
  };
}
