#!/bin/bash

REFERENCE_DIR="$(cd "$(dirname "$0")" && pwd)"

repos=(
    "https://github.com/php-lsys/token-monitor"
    "https://github.com/soulduse/ai-token-monitor"
    "https://github.com/BerriAI/litellm"
    "https://github.com/QuantumNous/new-api"
    "https://github.com/tensorzero/tensorzero"
    "https://github.com/browser-use/browser-use"
    "https://github.com/router-for-me/CLIProxyAPI"
    "https://github.com/CherryHQ/cherry-studio"
    "https://github.com/farion1231/cc-switch"
    "https://github.com/higress-group/higress"
)

echo "Starting to clone repositories into $REFERENCE_DIR..."

for repo in "${repos[@]}"; do
    repo_name=$(basename "$repo")
    target_dir="$REFERENCE_DIR/$repo_name"

    if [ -d "$target_dir" ]; then
        echo "Repository $repo_name already exists, skipping..."
    else
        echo "Cloning $repo..."
        git clone "$repo" "$target_dir"
        if [ $? -eq 0 ]; then
            echo "Successfully cloned $repo_name"
        else
            echo "Failed to clone $repo"
        fi
    fi
    echo "---"
done

echo "Done!"