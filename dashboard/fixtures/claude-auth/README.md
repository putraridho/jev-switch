SYNTHETIC outputs of `claude auth status`. Only the key NAMES follow the real command
(DESIGN 3.1: `loggedIn`, `planTier`, `subscriptionType`, `email`, `orgId`, `orgName`,
`configDirectory`); every value is invented. The e-mail is `canary-email@example.invalid`,
the org id is the label-shaped canary `jevcanary_9f3c1b7e` and the org name holds
`JEVCANARY_9f3c1b7e_DO_NOT_SURFACE`: none of them may appear in anything parsed from here.

- `logged_in.json`: `planTier` `max_5x` (and `subscriptionType` `max`).
- `logged_in_subscription_only.json`: no `planTier`; `subscriptionType` `Team-Premium 5x`,
  which normalises to the label `team_premium_5x`.
- `logged_out.json`: `loggedIn` false.
- `text_form.txt`, `text_form_logged_out.txt`: the non-JSON form, for the text rule of
  DESIGN 8.6 (logged in with no plan; logged out).
