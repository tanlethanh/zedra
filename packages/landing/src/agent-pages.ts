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
        q: "Is this different from Claude's built-in Remote Control?",
        a: "Claude Remote Control mirrors one session. Zedra adds the workspace around it: terminal, files, editor, diffs, markdown, and support for other agents.",
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
        q: "Can I run Codex and Claude Code side by side?",
        a: "Yes. Each project directory is a Zedra workspace, and each workspace can run whatever agents you start in its terminals.",
      },
    ],
  },
  {
    slug: "opencode",
    name: "OpenCode",
    title: "Run OpenCode from your phone: Zedra",
    description:
      "Remote control for OpenCode with a phone workspace: terminal, editor, markdown, git diff, and push notifications over a direct encrypted tunnel.",
    intro:
      "OpenCode keeps working after you leave your desk. Zedra gives your phone a remote workspace for the machine where OpenCode runs: answer prompts, steer the task, read the code, and review the diff before anything lands.",
    setupCmd: "zedra setup opencode",
    startNote: "In OpenCode, reload skills and run",
    startCmd: "/zedra-start",
    faq: [
      {
        q: "Can I respond to OpenCode prompts from my phone?",
        a: "Yes. The terminal is interactive, so OpenCode prompts render on your phone. `zedra setup opencode` also installs hooks for waiting notifications.",
      },
      {
        q: "Do I need an extra subscription or API key?",
        a: "No. Zedra controls the OpenCode already running on your machine, with your existing providers and login.",
      },
      {
        q: "Does my code pass through Zedra's servers?",
        a: "No. OpenCode and your code stay on your machine. Zedra connects directly when possible and uses encrypted relay fallback when networks block the path.",
      },
    ],
  },
];
