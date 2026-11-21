use crate::domain::worker_command::worker_may_run;

#[test]
fn a_worker_runs_read_only_and_project_commands_inside_its_folder() {
    for command in [
        "ls -la src",
        "cat Cargo.toml",
        "grep -rn TODO src",
        "git status",
        "git diff",
        "git log --oneline -5",
        "git show HEAD",
        "cargo test --quiet",
        "cargo check",
        "npm test",
        "sed -n 10,40p src/main.rs",
        "git diff | head -50",
        "wc -l src/lib.rs",
        "git log --oneline HEAD~3",
        "git diff origin/main..HEAD",
    ] {
        assert!(worker_may_run(command), "{command}");
    }
}

#[test]
fn a_worker_never_commits_uses_the_network_or_runs_what_a_person_must_allow() {
    for command in [
        "git add .",
        "git commit -m x",
        "git push",
        "npm view react",
        "docker ps",
        "kubectl get pods",
        "brew search x",
        "curl https://example.com",
        "rm -rf target",
        "python3 x.py",
        "sh -c ls",
        "find . -delete",
        "echo $HOME",
        "cat a > b",
    ] {
        assert!(!worker_may_run(command), "{command}");
    }
}

#[test]
fn a_worker_stays_inside_its_worktree() {
    for command in [
        "cat /etc/passwd",
        "cat ~/.ssh/id_rsa",
        "ls ~",
        "cat ../secret",
        "cat src/../../x",
        "ls /",
        "grep -r key /home",
        "git diff --no-index /etc/hosts a",
        "grep --file=/etc/x y",
        "git -C/tmp status",
        "head -n5 x/../../y",
        "cat ~root/x",
        "ls ..",
    ] {
        assert!(!worker_may_run(command), "{command}");
    }
}
