const decoder = new TextDecoder();

async function run(cmd: string[]): Promise<string> {
  const process = new Deno.Command(cmd[0], {
    args: cmd.slice(1),
    stdout: "piped",
    stderr: "piped",
  });
  const { stdout } = await process.output();
  return decoder.decode(stdout).trim();
}

function isReleaseOnlyCommit(msg: string): boolean {
  return (
    msg.startsWith("chore(release):") ||
    msg.startsWith("release:") ||
    /^Merge pull request #\d+ from .+\/release\//.test(msg)
  );
}

async function main() {
  const latestTag = await run(["git", "describe", "--tags", "--abbrev=0"]).catch(() => "");

  const range = latestTag ? `${latestTag}..HEAD` : "HEAD";
  const log = await run(["git", "log", range, "--pretty=format:%s"]);

  if (!log) {
    Deno.exit(0);
  }

  // Ignore release automation commits so merging a release PR (and the
  // Auto Release run it triggers on master) does not open another release PR.
  const commits = log.split("\n").filter((msg) => !isReleaseOnlyCommit(msg));
  if (commits.length === 0) {
    Deno.exit(0);
  }

  let bump: "major" | "minor" | "patch" = "patch";

  for (const msg of commits) {
    if (msg.includes("!:") || msg.includes("BREAKING CHANGE")) {
      bump = "major";
      break;
    }
    if (msg.startsWith("feat")) {
      bump = "minor";
    }
  }

  let major = 0, minor = 1, patch = 0;
  if (latestTag) {
    const match = latestTag.replace(/^v/, "").match(/^(\d+)\.(\d+)\.(\d+)/);
    if (match) {
      major = parseInt(match[1]);
      minor = parseInt(match[2]);
      patch = parseInt(match[3]);
    }
  }

  switch (bump) {
    case "major":
      major++;
      minor = 0;
      patch = 0;
      break;
    case "minor":
      minor++;
      patch = 0;
      break;
    case "patch":
      patch++;
      break;
  }

  const runNumber = Deno.env.get("GITHUB_RUN_NUMBER") ?? "1";
  const date = new Date().toISOString().slice(0, 10).replace(/-/g, "");
  const version = `${major}.${minor}.${patch}-alpha.${date}.${runNumber}`;

  const out = Deno.env.get("GITHUB_OUTPUT");
  if (out) {
    await Deno.writeTextFile(out, `version=${version}\n`, { append: true });
  }
  console.log(version);
}

main();
