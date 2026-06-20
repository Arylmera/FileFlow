# Product

## Register

product

## Users

A solo photographer (the developer) on macOS. They shoot to SD cards and export
edits from Lightroom, and want ingest + Photos import to happen automatically in
the background. They open this window only to configure rules or check what
happened — glancing at a menu-bar utility between shoots, not a place they linger.

## Product Purpose

FileFlow is a resident menu-bar app that (1) copies a recognised SD card's photos
into a per-card dated destination, verifies every copy, then optionally wipes and
ejects the card, and (2) imports a Lightroom export folder into Apple Photos. The
window is a control panel: configure card/Lightroom rules, trigger imports
manually, and read an activity log. Success = the user trusts it to run unattended
and rarely needs the window.

## Brand Personality

Quiet, dependable, native. Three words: trustworthy, unobtrusive, precise. It
should feel like a first-party macOS utility (Disk Utility, the Photos importer),
not a branded SaaS product. The user's task and data are the focus; the chrome
recedes.

## Anti-references

- Consumer SaaS dashboards with gradients, hero-metric cards, marketing flourish.
- Cross-platform Electron apps that ignore macOS conventions.
- Anything playful or attention-seeking — this is a background tool handling
  irreplaceable photos; it must read as careful, not cute.

## Design Principles

1. The tool disappears into the task — earned familiarity over novelty.
2. Native macOS feel: system blue accent, system font stack, standard controls.
3. State is always legible — watcher status, destination reachability, what just happened.
4. Restraint: neutral surfaces, one accent, color reserved for state and primary actions.
5. Safety reads as calm: destructive settings (cleanup/eject) are clear, never alarming.

## Accessibility & Inclusion

- Dark and light mode, system-driven.
- WCAG AA contrast for text and controls.
- Visible keyboard focus on every interactive element; full keyboard navigation.
- Respect `prefers-reduced-motion`.
