# Shogi Game Specification

## Overview

You are to implement a program that allows playing shogi. Everything runs on the CLI. You must implement shogi piece movements, promotion, illegal moves, and repetition (sennichite). However, you do not need to implement rules related to entering king (nyūgyoku).

Implement **Player vs CPU** and **CPU vs CPU** modes, and you are to design the algorithm that the CPU uses. The CPU must always make legal moves according to the rules, and if it cannot do so (i.e., it is checkmated), it must resign.

The game ends when either the player or the CPU resigns, and the program then terminates.

## Specifications

- Overall, adopt an efficient management approach.
- Use **Rust** as the development language.
- Represent the shogi board using **bitboards**. Represent pieces in hand also using **bit-based representations**.
- Implement **three CPU strength levels** (weak, normal, strong), differentiated by **search depth** (details of the search are specified later).
- You may choose the method by which the player specifies moves via standard input. Use a format that is easy to parse.
- Use a **time control with main time and byoyomi**:
  - Both sides have **10 minutes** of thinking time.
  - Once main time is exhausted, each move must be made within **30 seconds**.
- When a move is decided, both the player and the CPU must output the move they played to the CLI.
- On the CLI, display a **simplified shogi board diagram** that reproduces the position up to the most recent move.

## Search Engine

- You are also to implement the **search engine** yourself.
- Use the **alpha-beta pruning algorithm**.
- Evaluation values for each piece should be **fixed**.
- Design and implement the function that determines the **evaluation values**.

