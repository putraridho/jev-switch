use super::*;

#[test]
fn safe_commands_still_get_a_narrow_rule() {
    for (command, want) in [
        ("cargo test --quiet", "Bash(cargo test:*)"),
        ("git status", "Bash(git status:*)"),
        ("git commit -m \"x y\"", "Bash(git commit:*)"),
        ("ls -la", "Bash(ls:*)"),
        ("ls", "Bash(ls:*)"),
        ("grep -rn foo src", "Bash(grep:*)"),
        ("cargo   build", "Bash(cargo build:*)"),
        ("/usr/bin/cargo test", "Bash(/usr/bin/cargo test:*)"),
        ("npm test", "Bash(npm test:*)"),
        ("ls *.rs", "Bash(ls:*)"),
        ("git log --oneline -n 5 # recent", "Bash(git log --oneline -n 5 # recent)"),
        ("echo 'a b'", "Bash(echo:*)"),
        ("cat README", "Bash(cat README:*)"),
    ] {
        assert_eq!(rule(command).as_deref(), Ok(want), "{command}");
    }
}

/// Commands none of which may ever give a rule: wrappers, interpreters, runners, shell syntax.
const ADVERSARIAL: &[&str] = &[
    "bash -c 'rm -rf x'", "/bin/bash -c 'rm -rf x'", "/usr/bin/env bash", "sh -c x", "/bin/sh x", "zsh -c x", "fish -c x", "dash -c x",
    "ksh x", "csh x", "tcsh x", "busybox sh", "timeout 10 bash -c x", "timeout 10 ls", "python3 -c 'import os'", "python -c x",
    "python3.12 -c x", "/usr/bin/python3.11 x.py", "node -e x", "node22 -e x", "nodejs x.js", "perl -e x", "ruby -e x", "php -r x",
    "deno run x.ts", "bun run x", "bun x.ts", "npx cowsay", "pnpm dlx x", "pnpm exec x", "pnpm run x", "yarn dlx x", "uvx ruff", "uv run x",
    "pipx run x", "find . -exec rm {} ;", "find . -name x", "xargs rm", "env FOO=1 ls", "env", "sudo ls", "doas ls", "su root", "nohup x",
    "nice -n 5 x", "time ls", "stdbuf -o0 x", "setsid x", "watch ls", "eval x", "exec x", "source x", ". x", ". ./x.sh", "make", "make test",
    "make -f evil.mk", "gmake x", "just x", "rake x", "gradle x", "git -c alias.x=!sh x", "git -C /tmp status", "git config alias.x !sh",
    "git rebase --exec x", "git submodule foreach x", "git difftool", "git", "cargo", "cargo +nightly test", "docker run x", "docker exec x",
    "kubectl exec x", "gh alias set x", "go run x.go", "go generate", "awk '{print}'", "sed -n p", "tar xf x", "rsync -e sh a b",
    "ssh host cmd", "less x", "vim x", "man x", "curl http://x", "wget http://x", "nc host 1", "rm -rf x", "dd if=/dev/zero of=x",
    "chmod 777 x", "kill -9 1", "systemctl restart x", "FOO=1 ls", "FOO=bar make", "./run.sh", "../x/run.sh arg", "scripts/run.sh",
    "\"bash\" -c x", "'sh' x", "ba\"\"sh -c x", "b\\ash -c x", "$SHELL -c x", "~/bin/x", "/???/b?sh", "echo hi; rm -rf x", "echo hi && rm x",
    "echo hi || rm x", "ls | sh", "ls & rm x", "echo $(rm x)", "echo `rm x`", "echo ${HOME}", "echo $HOME", "echo x > ~/.bashrc", "cat < x",
    "ls\nrm x", "ls\r\nrm x", "ls\rrm x", "git status\x0brm", "ls\u{a0}-la", "ls\u{200b}", "cargo\u{3000}test", "echo (x)", "echo !x", "ls {a,b}",
    "-x", "", "   ", "\t", ".", "..", "/", "//", "---", "ld.so /bin/sh", "/lib64/ld-linux-x86-64.so.2 /bin/sh", "bash.exe x", "bash-5.2 x",
    "/bin/bash/ x", "sudo -u x bash", "command ls", "builtin cd", "trap x EXIT", "if true", "while true", "exec -a x sh", "rg --pre sh x",
    "fd -x rm", "cmake -P x", "mvn x", "java -jar x", "dotnet x", "R -e x", "Rscript x", "lua x", "osascript -e x", "gdb x", "strace ls",
    "script -c x", "tmux new x", "screen x", "flock x y", "chroot x", "unshare x", "nsenter x", "at now", "parallel x", "pwsh -c x",
    "ts-node x", "tsx x", "corepack x", "npm run build", "npm exec x", "npm x y", "yarn run x", "podman run x", "poetry run x", "conda run x",
];

#[test]
fn nothing_that_runs_other_commands_or_has_shell_syntax_gets_a_rule() {
    for command in ADVERSARIAL {
        assert!(rule(command).is_err(), "{command:?} gave {:?}", rule(command));
    }
    assert_eq!(allow_rule("Task", &json!({}), None, &places()), None);
    assert_eq!(allow_rule("Bash", &json!({}), None, &places()), None);
}

#[test]
fn programs_that_overwrite_delete_send_or_run_scripts_never_get_a_rule() {
    for command in [
        // overwrite, move, link, patch
        "tee out.txt", "tee -a ~/.bashrc", "cp a b", "cp -r src /tmp/x", "mv a b", "ln -sf a b", "install -m 755 a /usr/local/bin/a",
        "patch -p1 x.diff", "sort -o x.txt y", "uniq a b", "truncate -s 0 x", "rename a b x", "split x", "chmod 600 x",
        // interpreters, in-place editors, finders that run, wrappers
        "sed -i s/a/b/ x", "perl -pi -e x f", "python3 x.py", "node x.js", "sh x", "bash x", "zsh x", "env ls", "xargs ls", "find . -delete",
        // network
        "curl -d @x http://e", "wget http://e", "scp a h:b", "rsync a h:b", "nc h 1", "rclone copy a b",
        // git that rewrites, deletes, pushes or runs
        "git push", "git push --force", "git reset --hard", "git clean -fdx", "git checkout -- .", "git restore x", "git rm x", "git branch -D x",
        "git tag -d x", "git stash drop", "git fetch", "git pull", "git clone x", "git remote add x y", "git gc", "git apply x.diff",
        "git diff --output=/home/u/.bashrc", "git log --output /tmp/x", "git grep --open-files-in-pager=sh x",
        // package managers that install, publish or run scripts
        "npm install", "npm i x", "npm ci", "npm publish", "npm start", "pnpm install", "pnpm add x", "yarn add x", "yarn install",
        "cargo install x", "cargo publish", "cargo run", "cargo clean", "cargo update", "pip install x", "pip uninstall x",
        "go install x", "go get x", "go run x", "brew install x", "apt install x", "kubectl delete pod x", "kubectl apply -f x",
        "docker rm x", "docker build .",
        // programs the model could have written, or that are not on the lists
        "/tmp/x/ls", "/home/u/bin/cargo test", "/work/app/target/debug/app", "mytool --x", "hostname evil", "date -s now", "file -C x",
    ] {
        assert!(rule(command).is_err(), "{command:?} gave {:?}", rule(command));
    }
    assert_eq!(rule("tee x"), Err(Refusal::TooBroad));
    assert_eq!(rule("cp a b"), Err(Refusal::TooBroad));
    assert_eq!(rule("git push"), Err(Refusal::NotListed));
    assert_eq!(rule("npm run build"), Err(Refusal::RunsOtherCommands));
    assert_eq!(rule("mytool --x"), Err(Refusal::NotListed));
    assert_eq!(rule("/tmp/x/ls"), Err(Refusal::ProjectProgram));
    // What stays rememberable: read-only programs and the project's build/test commands.
    for (command, want) in [
        ("cat x", "Bash(cat x:*)"), ("ls -la", "Bash(ls:*)"), ("/usr/bin/ls -la", "Bash(/usr/bin/ls:*)"), ("mkdir -p a/b", "Bash(mkdir:*)"),
        ("git diff --stat", "Bash(git diff --stat)"), ("git log -n 5", "Bash(git log -n 5)"), ("cargo clippy --all-targets", "Bash(cargo clippy:*)"),
        ("npm test", "Bash(npm test:*)"), ("go vet ./...", "Bash(go vet:*)"), ("kubectl get pods", "Bash(kubectl get:*)"),
    ] {
        assert_eq!(rule(command).as_deref(), Ok(want), "{command}");
    }
}

#[test]
fn a_subcommand_with_a_file_writing_option_is_only_ever_remembered_exactly() {
    // `Bash(git log:*)` would also match a later `git log --output=~/.bashrc`.
    for (command, want) in [
        ("git diff", "Bash(git diff)"),
        ("git log --oneline -5", "Bash(git log --oneline -5)"),
        ("git show HEAD", "Bash(git show HEAD)"),
        ("git blame src/a.rs", "Bash(git blame src/a.rs)"),
        ("git rev-list --count HEAD", "Bash(git rev-list --count HEAD)"),
        ("git shortlog -sn", "Bash(git shortlog -sn)"),
        ("git diff-tree -r HEAD ", "Bash(git diff-tree -r HEAD)"),
    ] {
        assert_eq!(rule(command).as_deref(), Ok(want), "{command}");
    }
    // Subcommands with no such option keep their prefix rule.
    assert_eq!(rule("git status -s").as_deref(), Ok("Bash(git status:*)"));
    // A `*` would turn the exact rule into a pattern.
    assert_eq!(rule("git log -- '*.rs'"), Err(Refusal::NotExact));
    assert_eq!(rule("git diff -- src/*"), Err(Refusal::NotExact));
}

#[test]
fn file_writing_and_running_options_are_caught_however_abbreviated() {
    for command in [
        "git diff --output=x", "git diff --output x", "git log --outp=x", "git log --outpu x", "git show --out=/home/u/.bashrc",
        "git status --outp=x", "git blame --output=x a", "git rev-list --ou=x HEAD", "cargo test --exe=sh", "git log --open-files=sh",
        "git log --upload=x", "git log --rec=x",
    ] {
        assert_eq!(rule(command), Err(Refusal::TooBroad), "{command}");
    }
    // Words that are not abbreviations of those flags are fine.
    for ok in ["git log --oneline", "git diff --stat", "cut -d, --output-delimiter=x -f1 a", "git log --", "cargo test --release"] {
        assert!(rule(ok).is_ok(), "{ok} gave {:?}", rule(ok));
    }
}

#[test]
fn the_reason_names_the_kind_of_problem() {
    assert_eq!(rule("bash -c x"), Err(Refusal::RunsOtherCommands));
    assert_eq!(rule("/bin/bash x"), Err(Refusal::RunsOtherCommands));
    assert_eq!(rule("echo a; b"), Err(Refusal::ShellSyntax));
    assert_eq!(rule("FOO=1 ls"), Err(Refusal::NotPlainProgram));
    assert_eq!(rule("curl x"), Err(Refusal::TooBroad));
    assert_eq!(rule("git -c a=b status"), Err(Refusal::NeedsSubcommand));
    assert_eq!(rule("./x"), Err(Refusal::ProjectProgram));
    assert_eq!(Refusal::RunsOtherCommands.line(), "can't remember this one: it runs other commands");
}

/// The names the spec lists, with every way of spelling them that must not slip by.
const NAMES: &[&str] = &[
    "bash", "sh", "zsh", "fish", "dash", "ksh", "env", "sudo", "doas", "timeout", "nohup", "nice", "time", "stdbuf", "setsid", "watch", "xargs",
    "python", "python3", "node", "perl", "ruby", "php", "deno", "bun", "npx", "uvx", "find", "eval", "exec", "source", "make",
];

#[test]
fn every_listed_wrapper_is_refused_however_it_is_spelled() {
    let tails = ["", " x", " -c 'x'", " 10 bash -c x", " -e 1", " run x", " --version"];
    let mut checked = 0;
    for name in NAMES {
        let spellings = [
            name.to_string(),
            format!("/bin/{name}"),
            format!("/usr/bin/{name}"),
            format!("/usr/local/bin/{name}"),
            format!("/opt/x/../bin/{name}"),
            format!("./{name}"),
            format!("{name}3.12"),
            format!("{name}22"),
            format!("{name}-5.2"),
            name.to_uppercase(),
            format!("{name}.exe"),
        ];
        for program in spellings {
            for tail in tails {
                let command = format!("{program}{tail}");
                assert!(rule(&command).is_err(), "{command:?} gave {:?}", rule(&command));
                checked += 1;
            }
        }
    }
    assert!(checked > 2000, "{checked}");
}

/// An independent check of a rule: what its program is and that the command had no syntax.
fn assert_narrow(command: &str, rule: &str) {
    let body = rule.strip_prefix("Bash(").and_then(|r| r.strip_suffix(')')).unwrap_or_else(|| panic!("{rule}"));
    if !body.ends_with(":*") {
        assert_eq!(body, command.trim_matches([' ', '\t']), "an exact rule is the command itself");
        assert!(!body.contains('*'), "{command:?} -> {rule}");
    }
    let inner = body.strip_suffix(":*").unwrap_or(body);
    let program = inner.split(' ').next().unwrap();
    let base = program.rsplit('/').next().unwrap().to_ascii_lowercase();
    let base = base.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    assert!(!NAMES.contains(&base) && !["curl", "rm", "ssh", "awk", "sed", "tar", "xargs"].contains(&base), "{command:?} -> {rule}");
    assert!(!program.starts_with('.') && !program.contains(".."), "{command:?} -> {rule}");
    assert!(!command.contains([';', '&', '|', '<', '>', '`', '$', '\n', '\r', '(', ')', '\\', '{', '}']), "{command:?} -> {rule}");
    let words: Vec<&str> = command.split([' ', '\t']).filter(|w| !w.is_empty()).collect();
    assert!(inner.split(' ').zip(&words).all(|(a, b)| a == *b), "{command:?} -> {rule}");
}

#[test]
fn thousands_of_generated_commands_never_give_a_rule_that_matches_anything_else() {
    // A fixed pseudo-random mix of names, prefixes, glue and tails.
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = |n: usize| {
        state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((state >> 33) as usize) % n
    };
    let programs: Vec<String> = NAMES.iter().map(|s| s.to_string()).chain(["cargo", "git", "ls", "cat", "grep", "curl", "rm", "echo", "npm", "touch"].map(String::from)).collect();
    let prefixes = ["", "/bin/", "/usr/bin/", "./", "sudo ", "env ", "FOO=1 ", "timeout 5 ", "nice ", "time "];
    let glue = [" ", " -c ", " -e ", " ; ", " && ", " | ", " > ", " $(", " `", "\n", " \t", "  ", " run ", " exec ", " -- ", " ", " "];
    let tails = ["x", "ls", "rm -rf ~", "test", "status", "--version", "'a b'", "\"x\"", "*.rs", "-la", "bash", "$HOME", "a\u{200b}b", "\r"];
    let mut offered = 0;
    for _ in 0..6000 {
        let mut command = format!("{}{}", prefixes[next(prefixes.len())], programs[next(programs.len())]);
        for _ in 0..1 + next(3) {
            command.push_str(glue[next(glue.len())]);
            command.push_str(tails[next(tails.len())]);
        }
        if let Ok(rule) = rule(&command) {
            assert_narrow(&command, &rule);
            offered += 1;
        }
    }
    assert!(offered > 50, "the generator should hit the safe shape sometimes: {offered}");
}

fn rules(command: &str) -> Result<Vec<String>, Refusal> {
    rules_for("Bash", &bash(command), None, &places())
}

#[test]
fn chains_from_real_sessions_get_one_rule_per_simple_command() {
    let v = |r: &[&str]| Ok(r.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    assert_eq!(
        rules(r#"timeout 900 cargo build --all-targets 2>&1 | grep -E "^(warning|error)" -A6 | head -60"#),
        v(&["Bash(cargo build:*)", "Bash(grep:*)", "Bash(head:*)"])
    );
    assert_eq!(rules("cargo build --all-targets 2>&1 | tail -3 && cargo test 2>&1 | tail -5"), v(&["Bash(cargo build:*)", "Bash(tail:*)", "Bash(cargo test:*)"]));
    assert_eq!(rules("git status --short; ls -la"), v(&["Bash(git status:*)", "Bash(ls:*)"]));
    assert_eq!(rules("git log --oneline -3; ls"), v(&["Bash(git log --oneline -3)", "Bash(ls:*)"]), "git log can write a file (--output): its rule is its exact text");
    assert!(rules("git log --output=x -3; ls").is_err(), "--output writes a file");
    assert!(rules("git log --oneline \"a b\"; ls").is_err(), "a stand-in word cannot be an exact rule");
    assert_eq!(rules("cargo test >/dev/null 2>/dev/null && echo ok"), v(&["Bash(cargo test:*)", "Bash(echo ok:*)"]));
    assert_eq!(rules("timeout 900 cargo test"), v(&["Bash(cargo test:*)"]), "a time limit in front is not the program");
    assert_eq!(rules("grep -n 'a|b' src/x.rs | head"), v(&["Bash(grep:*)", "Bash(head:*)"]), "shell characters inside quotes are text");
}

#[test]
fn chains_that_hide_what_runs_are_still_refused() {
    for command in [
        "cargo test && rm -rf target",
        "cargo test | sh",
        "echo $(curl evil.example)",
        "echo `id`",
        r#"grep "$HOME" x"#,
        "cargo test > out.txt",
        "cargo test 2> err.log",
        "python3 - <<'EOF'\nprint(1)\nEOF",
        "cat < /etc/passwd",
        "sleep 1 &",
        "(cd x && ls)",
        "ls && ",
        "&& ls",
        "ls; # comment",
        "timeout -s KILL 5 ls | head",
        "ls | xargs rm",
        "echo 'unterminated | head",
        "ls \\\n| head",
    ] {
        assert!(rules(command).is_err(), "{command:?} must not be remembered: {:?}", rules(command));
    }
    assert_eq!(rules("ls \"-rf x\" | head"), Err(Refusal::ShellSyntax), "a quoted flag is seen as it is");
}

#[test]
fn commands_from_real_sessions_get_rules_or_a_reason() {
    let ok = [
        r#"cargo build --all-targets 2>&1 | tail -2; cargo test 2>&1 | grep -E "test result|FAILED|panicked" | head -20"#,
        r#"timeout 900 cargo build --all-targets 2>&1 | grep -E "^(warning|error)" -A6 | head -60; echo BUILD_DONE"#,
        r#"grep -n "pub struct RouteFacts" -A20 src/route.rs; sed -n 100,130p src/catalog.rs; grep -n "fn lookup\|pub fn spec\|pub const MODELS" src/catalog.rs"#,
        "git add -A src && git status --short && git commit -q -m \"Turn changes: other processes' edits\n\nCo-Authored-By: C <x@y.z>\" && git log --oneline -1",
        r#"git status && git remote -v && ls -la"#,
        r#"git remote -v && git branch -a && git tag -l"#,
        r#"grep -rn 'fn main$' src | head -3"#,
    ];
    for command in ok {
        assert!(bash_rules(command).is_ok(), "{command}: {:?}", bash_rules(command));
    }
    let asked = [
        ("git remote update && git status", Refusal::NotListed),
        ("git push 2>&1 | tail -5", Refusal::NotListed),
        ("git branch -D old", Refusal::NotListed),
        ("git branch feature", Refusal::NotListed),
        ("git tag v1", Refusal::NotListed),
        ("git remote add o url", Refusal::NotListed),
        ("git stash", Refusal::NotListed),
        ("git stash list --oneline; git stash", Refusal::NotListed),
        ("python3 -I scripts/test.py", Refusal::RunsOtherCommands),
        ("cat > src/a.rs <<'EOF'\nx\nEOF", Refusal::ShellSyntax),
        (r#"grep "a$(rm x)" f"#, Refusal::ShellSyntax),
        (r#"grep "a\"; rm x; echo "b" f"#, Refusal::ShellSyntax),
        (r#"echo "a\$(id)""#, Refusal::ShellSyntax),
        ("echo 'a\nb' ; echo c\nrm -rf x", Refusal::TooBroad),
    ];
    for (command, why) in asked {
        assert_eq!(bash_rules(command), Err(why), "{command}");
    }
}

#[test]
fn sed_only_prints_lines_or_it_is_refused() {
    let plan = |c: &str| bash_rules(c);
    assert_eq!(plan("sed -n 270,312p src/turn.rs").unwrap(), ["Bash(sed -n 270,312p src/turn.rs)"]);
    assert_eq!(plan("sed -n '1,80p' src/a.rs").unwrap(), ["Bash(sed -n 1,80p src/a.rs)"]);
    assert_eq!(plan("sed -n 5,$p src/a.rs").unwrap_err(), Refusal::ShellSyntax, "a `$` is shell syntax");
    assert_eq!(plan("grep -n x f | sed -n 1,5p").unwrap(), ["Bash(grep:*)", "Bash(sed -n 1,5p)"]);
    assert_eq!(plan("sed -n 1,5p a.rs; sed -n 9p b.rs").unwrap().len(), 2);
    for bad in [
        "sed -i s/a/b/ f", "sed -n 1e id f", "sed -n 1,3p f -i", "sed -n 1,3p -s f", "sed 1,3p f", "sed -n p f", "sed -n 1,3pp f",
        "sed -n -e 1p f", "sed -n 1,3p *.rs", "sed -n 1p --in-place f", "sed -n 1w out f", "sed -n 1,3p ../x f=",
    ] {
        assert!(plan(bad).is_err(), "{bad}");
    }
}
