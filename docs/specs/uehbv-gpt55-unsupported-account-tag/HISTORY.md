# History

## Initial implementation

Production invocation `proxy-3048-1777115903668` showed a `gpt-5.5` request rejected by a ChatGPT OAuth account with an unsupported-model error. The routing behavior was updated to classify that condition as account-specific and recoverable by marking the account with a protected system tag and trying another eligible account.
