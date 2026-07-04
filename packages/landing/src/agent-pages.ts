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
    title: "Run Claude Code from your phone — Zedra",
    description:
      "Control Claude Code from your iPhone or Android phone with Zedra: a real terminal, code editor, git diffs, and push notifications over an end-to-end encrypted P2P tunnel. No VPN or port forwarding.",
    intro:
      "Claude Code keeps working after you leave your desk. Zedra connects your phone to the machine where Claude Code runs, in a real terminal — approve permission prompts, send follow-ups, and review the diff before anything lands.",
    setupCmd: "zedra setup claude",
    startNote: "In Claude Code, reload plugins and run",
    startCmd: "/zedra-start",
    faq: [
      {
        q: "Can I approve Claude Code permission prompts from my iPhone?",
        a: "Yes. The Zedra terminal is fully interactive, so permission prompts render on your phone and you answer them there. `zedra setup claude` also installs hooks that push a notification when Claude Code is waiting on you.",
      },
      {
        q: "Do I need an extra subscription or API key?",
        a: "No. Zedra controls the Claude Code you already run on your machine, with your existing login and plan. Zedra itself is free and MIT-licensed.",
      },
      {
        q: "Does my code pass through Zedra's servers?",
        a: "No. Claude Code and your code stay on your machine. The phone connects over a direct P2P tunnel with TLS 1.3 end-to-end encryption; when a direct path is blocked, a relay forwards encrypted packets it cannot read. Claude Code still talks to Anthropic's API as it normally does.",
      },
      {
        q: "What happens when my laptop sleeps?",
        a: "Claude Code runs on your machine, so a sleeping machine pauses the session. Keep the machine awake for long runs; Zedra reconnects and restores the terminal backlog when it comes back.",
      },
      {
        q: "Is this different from Claude's built-in Remote Control?",
        a: "Remote Control mirrors one Claude Code session into the Claude app. Zedra gives you a workspace around the session: a terminal that runs any agent, a file browser, a code editor, git diffs, and markdown preview — one app for your whole flow.",
      },
    ],
  },
  {
    slug: "codex",
    name: "Codex",
    title: "Run Codex CLI from your phone — Zedra",
    description:
      "Control OpenAI Codex CLI from your iPhone or Android phone with Zedra: a real terminal, code editor, git diffs, and push notifications over an end-to-end encrypted P2P tunnel. No VPN or port forwarding.",
    intro:
      "Codex keeps working after you leave your desk. Zedra connects your phone to the machine where Codex runs, in a real terminal — answer approval requests, steer the task, and review the diff before anything lands.",
    setupCmd: "zedra setup codex",
    startNote: "In Codex, reload skills and run",
    startCmd: "$zedra-start",
    faq: [
      {
        q: "Can I approve Codex commands from my phone?",
        a: "Yes. The Zedra terminal is fully interactive, so Codex approval prompts render on your phone and you answer them there. `zedra setup codex` installs hooks that push a notification when Codex is waiting on you.",
      },
      {
        q: "Do I need an extra subscription or API key?",
        a: "No. Zedra controls the Codex CLI you already run on your machine, with your existing login and plan. Zedra itself is free and MIT-licensed.",
      },
      {
        q: "Does my code pass through Zedra's servers?",
        a: "No. Codex and your code stay on your machine. The phone connects over a direct P2P tunnel with TLS 1.3 end-to-end encryption; when a direct path is blocked, a relay forwards encrypted packets it cannot read. Codex still talks to OpenAI's API as it normally does.",
      },
      {
        q: "Can I run Codex and Claude Code side by side?",
        a: "Yes. Each project directory is its own Zedra workspace, and each workspace runs whatever agents you start in its terminals. `zedra setup` with no argument installs hooks for every detected agent.",
      },
    ],
  },
  {
    slug: "opencode",
    name: "OpenCode",
    title: "Run OpenCode from your phone — Zedra",
    description:
      "Control OpenCode from your iPhone or Android phone with Zedra: a real terminal, code editor, git diffs, and push notifications over an end-to-end encrypted P2P tunnel. No VPN or port forwarding.",
    intro:
      "OpenCode keeps working after you leave your desk. Zedra connects your phone to the machine where OpenCode runs, in a real terminal — answer prompts, steer the task, and review the diff before anything lands.",
    setupCmd: "zedra setup opencode",
    startNote: "In OpenCode, reload skills and run",
    startCmd: "/zedra-start",
    faq: [
      {
        q: "Can I respond to OpenCode prompts from my phone?",
        a: "Yes. The Zedra terminal is fully interactive, so OpenCode prompts render on your phone and you answer them there. `zedra setup opencode` installs hooks that push a notification when OpenCode is waiting on you.",
      },
      {
        q: "Do I need an extra subscription or API key?",
        a: "No. Zedra controls the OpenCode you already run on your machine, with your existing providers and login. Zedra itself is free and MIT-licensed.",
      },
      {
        q: "Does my code pass through Zedra's servers?",
        a: "No. OpenCode and your code stay on your machine. The phone connects over a direct P2P tunnel with TLS 1.3 end-to-end encryption; when a direct path is blocked, a relay forwards encrypted packets it cannot read.",
      },
    ],
  },
];
