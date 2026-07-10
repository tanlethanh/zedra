export interface AgentPage {
  slug: string;
  name: string;
  title: string;
  description: string;
  intro: string;
  setupCmd: string;
  startNote: string;
  startCmd: string;
  faq: { q: string; a: string }[];
}

export const pages: AgentPage[] = [
  {
    slug: "claude-code",
    name: "Claude Code",
    title: "Run Claude Code from your phone: Zedra",
    description:
      "Remote control for Claude Code with a phone workspace: terminal, editor, markdown, git diff, and push notifications over a direct encrypted tunnel.",
    intro:
      "Claude Code keeps working after you leave your desk. Zedra gives your phone a remote workspace for the machine where Claude Code runs: approve prompts, send follow-ups, read the code, and review the diff before anything lands.",
    setupCmd: "zedra setup claude",
    startNote: "In Claude Code, reload plugins and run",
    startCmd: "/zedra-start",
    faq: [
      {
        q: "Can I approve Claude Code permission prompts from my iPhone?",
        a: "Yes. The terminal is interactive, so prompts render on your phone. `zedra setup claude` also installs hooks for waiting notifications.",
      },
      {
        q: "Do I need an extra subscription or API key?",
        a: "No. Zedra controls the Claude Code already running on your machine, with your existing login and plan.",
      },
      {
        q: "Does my code pass through Zedra's servers?",
        a: "No. Claude Code and your code stay on your machine. Zedra connects directly when possible and uses encrypted relay fallback when networks block the path.",
      },
      {
        q: "What happens when my laptop sleeps?",
        a: "Claude Code runs on your machine, so sleep pauses the session. When it wakes, Zedra reconnects and restores the terminal backlog.",
      },
      {
        q: "Why use Zedra instead of Claude's built-in mobile experience?",
        a: "The built-in experience is tied to one vendor and a mirrored view of the session. Zedra controls the Claude Code already running on your own machine — your login, tools, and repo, no hosted sandbox — and gives you the real interactive terminal plus files, editor, git diffs, and markdown. The same app also runs Codex, OpenCode, and other agents, so you are not locked to one vendor.",
      },
    ],
  },
  {
    slug: "codex",
    name: "Codex",
    title: "Run Codex CLI from your phone: Zedra",
    description:
      "Remote control for Codex CLI with a phone workspace: terminal, editor, markdown, git diff, and push notifications over a direct encrypted tunnel.",
    intro:
      "Codex keeps working after you leave your desk. Zedra gives your phone a remote workspace for the machine where Codex runs: answer approvals, steer the task, read the code, and review the diff before anything lands.",
    setupCmd: "zedra setup codex",
    startNote: "In Codex, reload skills and run",
    startCmd: "$zedra-start",
    faq: [
      {
        q: "Can I approve Codex commands from my phone?",
        a: "Yes. The terminal is interactive, so Codex approval prompts render on your phone. `zedra setup codex` also installs hooks for waiting notifications.",
      },
      {
        q: "Do I need an extra subscription or API key?",
        a: "No. Zedra controls the Codex CLI already running on your machine, with your existing login and plan.",
      },
      {
        q: "Does my code pass through Zedra's servers?",
        a: "No. Codex and your code stay on your machine. Zedra connects directly when possible and uses encrypted relay fallback when networks block the path.",
      },
      {
        q: "Why use Zedra instead of Codex's own apps?",
        a: "Zedra controls the Codex CLI already running on your machine — your login, tools, and repo, with no hosted sandbox. You get a real interactive terminal plus files, git diffs, and markdown, and the same app also runs Claude Code, OpenCode, and other agents, so you are not tied to one vendor.",
      },
      {
        q: "Can I run Codex and Claude Code side by side?",
        a: "Yes. Each project directory is a Zedra workspace, and each workspace can run whatever agents you start in its terminals.",
      },
    ],
  },
];
