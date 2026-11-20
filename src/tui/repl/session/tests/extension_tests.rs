//! Custom commands and recipes.

use super::*;

const ECHO_PROMPT: &str = "printf 'FIN:%s\\n' \"$(printf '%s' \"$FAKE_PROMPT\" | tr '\\n' ' ')\"";

fn write_ext(r: &Rig, rel: &str, text: &str) {
    let path = r.dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[tokio::test]
async fn a_custom_command_runs_its_prompt_with_the_arguments() {
    let mut r = rig("custom-cmd", ECHO_PROMPT);
    write_ext(&r, ".jev/commands/shout.md", "---\ndescription: Shout\n---\nSHOUT about $ARGUMENTS now");
    assert_eq!(run(&mut r, "/shout the parser").await, Flow::Continue);
    assert!(r.out.text().contains("SHOUT about the parser now"), "{}", r.out.text());
    assert_eq!(r.repl.log().turns()[0].prompt, "/shout the parser");
    // Still an unknown command when no file defines it.
    r.out.clear();
    run(&mut r, "/nothing-here").await;
    assert!(r.out.text().contains("unknown command /nothing-here"), "{}", r.out.text());
}

#[tokio::test]
async fn a_recipe_runs_its_steps_in_order_and_lists_with_recipe() {
    let mut r = rig("recipe", ECHO_PROMPT);
    write_ext(&r, ".jev/recipes/two.md", "---\ndescription: Two steps\n---\n## First\nstep-one for $1\n## Second\nstep-two");
    run(&mut r, "/recipe").await;
    assert!(r.out.text().contains("/two") && r.out.text().contains("2 steps"), "{}", r.out.text());
    r.out.clear();
    run(&mut r, "/recipe two alpha").await;
    let o = r.out.text();
    assert!(o.contains("step 1/2: First") && o.contains("step-one for alpha") && o.contains("step-two"), "{o}");
    let turns = r.repl.log().turns();
    assert_eq!((turns.len(), turns[0].prompt.as_str(), turns[1].prompt.as_str()), (2, "/two alpha [1/2]", "/two alpha [2/2]"));
    r.out.clear();
    run(&mut r, "/recipe missing").await;
    assert!(r.out.text().contains("no recipe named missing"), "{}", r.out.text());
}

#[tokio::test]
async fn a_recipe_stops_at_the_first_step_that_fails() {
    let mut r = rig("recipe-stop", "printf 'FAIL:boom\\n'; exit 1");
    write_ext(&r, ".jev/recipes/two.md", "## A\none\n## B\ntwo");
    run(&mut r, "/recipe two").await;
    assert_eq!(r.repl.log().turns().len(), 1, "the second step was not sent");
}
