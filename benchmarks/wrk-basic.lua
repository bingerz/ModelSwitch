-- wrk basic benchmark script for ModelSwitch
--
-- Usage:
--   wrk -t4 -c100 -d30s --script benchmarks/wrk-basic.lua \
--       --header "Authorization: Bearer YOUR_VIRTUAL_KEY" \
--       http://localhost:8080/v1/chat/completions
--
-- Parameters:
--   -t4   = 4 threads
--   -c100 = 100 concurrent connections
--   -d30s = 30 second duration

local payload = table.concat({
  '{"model":"deepseek-chat",',
  '"messages":[{"role":"user","content":"Hello"}],',
  '"max_tokens":10}',
})

wrk.method = "POST"
wrk.body = payload
wrk.headers["Content-Type"] = "application/json"
-- Authorization header should be passed via --header flag on CLI
