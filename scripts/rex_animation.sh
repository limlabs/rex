#!/bin/bash
# Rex mascot tail-wag animation preview
# Usage: ./scripts/rex_animation.sh

GREEN='\033[38;2;46;204;113m'
RESET='\033[0m'

# Hide cursor, restore on exit
tput civis
trap 'tput cnorm; exit' INT TERM

clear

while true; do
  # Frame 1: tail resting
  tput cup 4 0
  printf "${GREEN}        ▄████▄\n"
  printf "        █ ◦ █▀█▄\n"
  printf "  ▄▄▄▄▄▄█████▀▀\n"
  printf "    ▀▀▀▀██████\n"
  printf "        █▀ █▀${RESET}\n"
  sleep 0.4

  # Frame 2: tail tip flicks up
  tput cup 4 0
  printf "${GREEN}        ▄████▄\n"
  printf "        █ ◦ █▀█▄\n"
  printf "  ▀▄▄▄▄▄█████▀▀\n"
  printf "   ▀▀▀▀▀██████\n"
  printf "        █▀ █▀${RESET}\n"
  sleep 0.3
done
