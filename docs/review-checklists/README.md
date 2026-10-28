# Review checklists

Focused checklists for reviewing changes in areas where mistakes are expensive: mail correctness, security, architectural boundaries, and responsiveness.

They are written for human reviewers first. Coding agents are pointed at the same files, so a review does not depend on which tool ran it.

| Checklist | Apply when the change touches |
| --- | --- |
| [Mail domain and sync](mail-domain.md) | schema, sync deltas, op_log, threads, flags, message identity, UIDVALIDITY |
| [Security and privacy](security.md) | rendering, sanitization, CSP, capabilities, secrets, attachments, IPC, logging |
| [Architecture boundaries](architecture-boundaries.md) | modules, ports, adapters, commands, directory structure |
| [Frontend, UX, and responsiveness](frontend-ux.md) | screens, list virtualization, keyboard navigation, design tokens, perceived speed |
