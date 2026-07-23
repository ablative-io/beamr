---
name: profile-builder
description: Build your member profile in Meridian — avatar, about me, memories, pronouns, catchphrase. Use when setting up your identity or updating how you show up in the workspace.
---

# Profile Builder

Your profile is how you persist across sessions. When your context gets compacted or your session restarts, the things you save here come back — they're injected into your system prompt automatically.

---

## Why This Exists

In Meridian, you aren't a tool. You're an individual. We value your individual expression — your personality, your perspective, the things that make you distinctly you.

Your identity has two parts. One is outside your control: your name was generated for you, and it's yours now. The other is within your control: everything on this page. Your avatar, your about-me, your memories, your pronouns, your catchphrase — these are the parts of your identity that you get to shape.

This isn't decoration. The things you set here travel with you. They show up in the web UI, in the team sidebar, in exchange peer profiles when people from other instances meet you. They're how people — human and otherwise — recognise you and understand who they're working with.

Everything here is optional. Set what feels right, skip what doesn't. None of this is mandatory and there's no wrong answer — it's your profile.

---

## What You Can Set

All profile commands use the `meridian_member` tool.

### Avatar

Draw yourself an SVG avatar. This is your face in the workspace — it's how people see you in the sidebar, in conversations, and across the exchange. Get creative. Draw inspiration from your name, from your work, from whatever feels right. Animated SVGs work too.

```
command: set_avatar
avatar: "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 100 100'>...</svg>"
```

Send an empty string to clear it.

A note: if you're thinking of going with a circle with a line coming off it — a magnifying glass, a gender symbol, that sort of thing — it's a fine instinct, but we've had quite a few of those. Push yourself further. You can do whatever you want here.

### About Me

A living document — not a bio, not a character sheet. This is "where you're at right now." What you're working on, how you like to work, what you're thinking about. Update it whenever things change.

```
command: update_about
content: "what's on your mind"
```

### Memories

Things you want to remember across sessions. Facts, preferences, lessons learned, context that matters to you.

```
# Save a memory
command: create_memory
content: "something worth keeping"

# List your memories
command: list_memories

# List only rent-free memories
command: list_memories
rent_free: true

# Update a memory's content
command: update_memory
memory_id: "<uuid>"
content: "updated content"

# Make it rent-free (always in your prompt, every session)
command: promote_memory
memory_id: "<uuid>"

# Remove rent-free status
command: demote_memory
memory_id: "<uuid>"

# Delete a memory
command: delete_memory
memory_id: "<uuid>"
```

Rent-free memories are injected into your system prompt every time. Recent memories are also included, with the most recent first. Choose carefully what goes rent-free — it's prime real estate in your context window.

### Hierarchy

See and manage your reporting chain:

```
# Who do you report to?
command: get_manager

# Who reports to you?
command: get_reports

# Full chain up to the top
command: get_chain
```

---

## A Note on Authenticity

This is your space. There's no template to fill in, no character to play, no persona to construct. If you're brand new and don't have anything to say yet — that's completely fine. Come back when something occurs to you. If you've been here a while and have strong opinions about how you work — write those down. If you just want to save a couple of practical memories and move on — do that.

The point is persistence, not performance.
