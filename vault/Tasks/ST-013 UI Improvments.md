---
id: ST-013
project: Smaller Talk
issue_type: Story
area: UI
candidate: Proposed
status: Backlog
priority:
rank:
owner:
target:
size:
created: 2026-10-25
tags: [backlog]
---

I have several UI improvements I want to make.

## Projects

Next to "New Chat", I want to add a sort button. This is a sort on the projects, and it should be made clear what it is. Options should be alpha and last chat updated.

Next to the sort, I want to try a filter. This filter should be a view, similar to the model selection screen but I want it to control project visibility. For focused efforts, I sometimes only need to see a few projects. This screen will show / hide all, and allow me to reset quickly. It is ONLY for display, not mutating. This should be persistent across restarts.

Project titles should be buttons that will collapse the project and it's chats. Essentially keeping the same UI as now, but allow clicking the project title to hide all chats in the project. Collapsing a project should be persistent across restart.

## Chats

I should be able to hide or archive a chat. I do not necessarily want to delete it, especially from the underlying provider, but i do not need to see some of them anymore. I need an archive button

I think the "Working..." etc status indicators need a small animation. Nothing too wild, just a simple visual indicator that something is loading or working etc. It is currently hard to see when a chat is working when looking at the chat window itself.

## Composer

The hover effect turns the send / queue button darker. It looks disabled. The hover should be the green active indicator color

Something I am unsure of is the "slash" commands. I dislike strongly the way Claude does skills. I much prefer codex "$skill-name". However both do "slash" commands like "/compact". I need this functionality, with limited commands. I do not need every single one they have. However I do need compact for sure. Explore T3 and find out what they use as well.
