#!/bin/sh
#MISE description="Fail when a mise lockfile no longer matches its config"
# pre-push runs this. `mise lock --dry-run` writes and installs nothing, and
# prints [] when every tool has a current version in its lock; it does not see
# a missing platform entry or a missing download URL. `mise install --dry-run
# --locked`, run once for each lockfile_platforms entry, fails on those, as the
# locked installs of CI and of the shared secrets job do. It skips a tool
# already installed, so it runs against an empty data directory. Both calls
# also run against an empty global and system config directory, because a
# tool the pusher's own config pins has no entry in any project lock and would
# fail every profile; the repository is trusted by path instead, since the
# pusher's trusted_config_paths is dropped with that config.
#
# A dry run checks only the locks of the profiles MISE_ENV loads, so it runs
# once for each committed mise.<profile>.toml, which also checks mise.lock;
# a MISE_ENV the pusher's shell exports changes nothing. Under the hooks
# profile, mise would assign the pinned catalogs' task tools to that
# profile's lockfile, which nothing writes, so it is left out, and so is a
# machine's own mise.local.toml. Only tracked files count, so an untracked
# profile the push does not carry cannot fail it. hooks pieces always render
# mise.dev.toml.
platforms=$(mise settings get lockfile_platforms) || { echo "✘ mise settings get lockfile_platforms failed; see its error above"; exit 1; }
platforms=$(printf '%s' "$platforms" | tr -d '[]"' | tr ',' ' ')
[ -n "$platforms" ] || { echo "✘ lockfile_platforms is empty; list the platforms the locks record in mise.toml"; exit 1; }
data=$(mktemp -d) || { echo "✘ mktemp failed; see its error above"; exit 1; }
isolated_config=$(mktemp -d) || { echo "✘ mktemp failed; see its error above"; exit 1; }
trap 'rm -rf "$data" "$isolated_config"' EXIT
isolated() { env MISE_CONFIG_DIR="$isolated_config" MISE_SYSTEM_CONFIG_DIR="$isolated_config" MISE_TRUSTED_CONFIG_PATHS="$PWD" "$@"; }
configs=$(git ls-files -- 'mise.*.toml') || { echo "✘ git ls-files failed; see its error above"; exit 1; }
stale=0
for config in $configs; do
  profile=${config#mise.}
  profile=${profile%.toml}
  case $profile in
    hooks | local | *.local) continue ;;
  esac
  changes=$(isolated MISE_ENV=$profile mise lock --dry-run --json) || { echo "✘ mise lock --dry-run failed in the $profile profile; see its error above"; exit 1; }
  if [ "$changes" != "[]" ]; then
    printf '%s\n' "$changes"
    echo "✘ a mise lockfile of the $profile profile is stale: run MISE_ENV=$profile mise lock"
    stale=1
  fi
  for platform in $platforms; do
    # MISE_OS and MISE_ARCH make the dry run read the lock entry of that
    # platform, whichever machine pushes.
    if ! output=$(isolated MISE_DATA_DIR="$data" MISE_ENV=$profile MISE_OS=${platform%%-*} MISE_ARCH=${platform#*-} mise install --dry-run --locked 2>&1); then
      printf '%s\n' "$output"
      echo "✘ a mise lockfile of the $profile profile has no download URL for a tool on $platform: run MISE_ENV=$profile mise lock"
      stale=1
    fi
  done
done
exit "$stale"
