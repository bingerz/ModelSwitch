# ModelSwitch 性能基准测试

> 使用 k6 和 wrk 对 ModelSwitch LLM 网关进行压力测试与基线测试。

## 目录

1. [环境准备](#环境准备)
2. [k6 企业级负载测试](#k6-企业级负载测试)
3. [wrk 基线测试](#wrk-基线测试)
4. [结果解读](#结果解读)
5. [调优建议](#调优建议)

---

## 环境准备

### 前置条件

- ModelSwitch 服务已部署并可访问
- 至少一个可用的虚拟密钥 (Virtual Key)
- 上游 LLM 通道 (如 DeepSeek) 已正确配置

### 安装 k6

```bash
# macOS
brew install k6

# Linux (Debian/Ubuntu)
sudo gpg -k
sudo gpg --no-default-keyring --keyring /usr/share/keyrings/k6-archive-keyring.gpg --keyserver hkp://keyserver.ubuntu.com:80 --recv-keys C5AD17C747E3415A36442D57D77B930C9EF15EF4
echo "deb [signed-by=/usr/share/keyrings/k6-archive-keyring.gpg] https://dl.k6.io/deb stable main" | sudo tee /etc/apt/sources.list.d/k6.list
sudo apt update && sudo apt install k6

# Docker
docker run --rm -i grafana/k6 run - < benchmarks/enterprise-load-test.yml
```

### 安装 wrk

```bash
# macOS
brew install wrk

# Linux (Debian/Ubuntu)
sudo apt install build-essential libssl-dev git
git clone https://github.com/wg/wrk.git
cd wrk && make
sudo cp wrk /usr/local/bin/
```

---

## k6 企业级负载测试

### 测试场景

模拟 1000 并发用户的完整负载周期:

| 阶段 | 持续时间 | 目标用户数 | 说明 |
|------|----------|------------|------|
| Ramp Up 1 | 30 秒 | 100 | 逐步加压 |
| Ramp Up 2 | 1 分钟 | 500 | 中等负载 |
| Ramp Up 3 | 2 分钟 | 1000 | 峰值负载 |
| Sustained | 5 分钟 | 1000 | 持续峰值 |
| Ramp Down | 30 秒 | 0 | 逐步减压 |

### 阈值标准

- 99% 请求响应时间 < 500ms
- 错误率 < 5%

### 运行测试

```bash
# 设置环境变量
export BASE_URL=http://localhost:8080
export VIRTUAL_KEY=your-virtual-key-here

# 运行测试
k6 run benchmarks/enterprise-load-test.yml

# 通过 Docker 运行
docker run --rm -i \
  -e BASE_URL=http://host.docker.internal:8080 \
  -e VIRTUAL_KEY=your-virtual-key-here \
  grafana/k6 run - < benchmarks/enterprise-load-test.yml
```

### 输出到 JSON (用于 CI/CD)

```bash
k6 run --out json=results.json benchmarks/enterprise-load-test.yml
```

---

## wrk 基线测试

wrk 适合快速获取基线性能数据, 测试开销更低。

### 基本测试

```bash
# 100 并发连接, 30 秒
wrk -t4 -c100 -d30s \
  --script benchmarks/wrk-basic.lua \
  --header "Authorization: Bearer YOUR_VIRTUAL_KEY" \
  http://localhost:8080/v1/chat/completions
```

### 高并发测试

```bash
# 500 并发连接, 60 秒
wrk -t8 -c500 -d60s \
  --script benchmarks/wrk-basic.lua \
  --header "Authorization: Bearer YOUR_VIRTUAL_KEY" \
  http://localhost:8080/v1/chat/completions
```

### 参数说明

| 参数 | 说明 |
|------|------|
| `-tN` | 线程数 (建议 = CPU 核心数) |
| `-cN` | 并发连接数 |
| `-dNs` | 持续时间 (秒) |

---

## 结果解读

### k6 关键指标

| 指标 | 含义 | 目标值 |
|------|------|--------|
| `http_req_duration` | 请求总耗时 | p(99) < 500ms |
| `http_req_failed` | 失败请求比率 | < 5% |
| `iterations` | 完成的迭代次数 | 越高越好 |
| `vus` | 当前活跃虚拟用户数 | 应跟随 stages 曲线 |
| `http_reqs` | 总请求数 | — |

### wrk 关键指标

| 指标 | 含义 |
|------|------|
| `Latency` | 延迟分布 |
| `Req/Sec` | 每秒请求数 |
| `Transfer/sec` | 每秒传输数据量 |

### 预期性能参考

基于单机部署 (1 CPU / 512MB):

| 并发数 | 预期 QPS | p(99) 延迟 |
|--------|----------|------------|
| 100 | 80-100 | < 200ms |
| 500 | 300-500 | < 400ms |
| 1000 | 500-800 | < 500ms |

> 注意: 实际性能受上游 LLM API 延迟影响。上述数据假设上游响应 < 100ms。

---

## 调优建议

### 如果 p(99) 超标

1. **增大连接池**: 在 `config.toml` 中提高 `http_pool_size` (默认 8, 建议 16-32)
2. **启用缓存**: 确认 `cache_mode = "on"`
3. **增加资源**: 提升 Docker CPU/内存限制
4. **优化上游**: 选择延迟更低的上游通道

### 如果错误率超标

1. **检查虚拟密钥配额**: 确认配额是否耗尽
2. **检查通道熔断状态**: 查看 `GET /api/channels/status`
3. **检查预算限制**: 查看 `GET /api/provider-budgets`
4. **查看日志**: `docker compose logs modelswitch | grep ERROR`

### 如果 QPS 偏低

1. **增加线程**: wrk `-t` 参数设为 CPU 核心数
2. **减少 sleep**: 调整测试脚本中的 `sleep(1)` 为更小值
3. **检查限速**: 确认虚拟密钥的 `rpm_limit` 设置合理
4. **排除网络瓶颈**: 确保测试客户端与服务在同一网络
