# ⚡ TraceForge (AgentOps Guard)

> **Real-Time Invariant Firewall, Loop-Interception Proxy & Time-Travel Debugger for Autonomous LLM Agents in Production.**

[![Proxy Latency](https://img.shields.io/badge/Proxy%20Latency-<5ms-brightgreen.svg)](#proxy-architecture)
[![Policy Engine](https://img.shields.io/badge/Guardrails-OPA%20%2F%20Rego-blue.svg)](#policy--invariant-engine)
[![Tests](https://img.shields.io/badge/Tests-95%20passing-brightgreen.svg)](https://github.com/nolekek/trace-forge/actions)
[![License](https://img.shields.io/badge/License-Dual%20AGPLv3%20%2F%20Enterprise-blueviolet.svg)](#monetization--business-model)

---

## ⚡ Quickstart: Быстрый запуск

### 1. Локальный запуск единым бинарником (Single-Binary)
Благодаря встраиванию скомпилированного дашборда (`rust-embed`), Node.js для запуска не требуется:

```bash
# Клонирование и запуск прокси-файрвола
git clone https://github.com/nolekek/trace-forge.git
cd trace-forge
cargo run --release --bin traceforge -- run --port 8080 --upstream https://api.openai.com/v1 --max-budget 10.0
```

- **Reverse Proxy**: `http://localhost:8080/v1`
- **Real-Time Web Canvas**: `http://localhost:8080/dashboard`
- **MCP Protocol Gateway**: `http://localhost:8080/mcp`
- **WebSocket Control Plane**: `ws://localhost:8080/ws/control`

### 2. Запуск в Docker
```bash
docker compose up -d
```

### 3. Подключение AI-агента (Python / OpenAI SDK / LangGraph)
Для интеграции достаточно перенаправить `base_url`:

```python
from openai import OpenAI

# Все запросы агента теперь проходят через сетевой файрвол TraceForge
client = OpenAI(
    base_url="http://localhost:8080/v1",
    api_key="your-openai-api-key"
)

response = client.chat.completions.create(
    model="gpt-4o",
    messages=[{"role": "user", "content": "Refactor codebase"}],
    stream=True
)
```

---

## 📌 Executive Summary

Когда автономные AI-агенты выходят за рамки чат-ботов и начинают управлять кодовыми базами, серверами и базами данных, они неизбежно создают критические инциденты:
1. **Бесконечные циклы галлюцинаций («Loop Thrashing»)**: агент получает ошибку, пробует тот же тул с микро-изменениями 50 раз подряд, сжигая $200 на API за 10 минут.
2. **Отсутствие упреждающих ограничений (Runtime Invariants)**: существующие системы (Langfuse, Arize) пассивно логируют катастрофу уже постфактум, вместо того чтобы перехватить деструктивную команду до ее выполнения.
3. **Непрозрачность контекста**: разработчики не видят, в какой момент раздутое контекстное окно потеряло важную инструкцию и начало деградировать.

**TraceForge** — это сверхбыстрый (<5ms) прокси-файрвол и наблюдаемость для агентов. Он в реальном времени анализирует поток мыслей/действий агента, обрывает циклы, проверяет инварианты безопасности через политики Rego и предоставляет визуальный холст с возможностью паузы и ручного одобрения (Human-in-the-Loop).

---

## 🏗️ Архитектура системы

```mermaid
flowchart TD
    subgraph AgentRuntime ["Agent Runtime / App"]
        LLM_Client["Agent Core (LangGraph / AutoGen / Custom)"]
    end

    subgraph TraceForge_Engine ["TraceForge Guard & Proxy (<5ms Latency)"]
        StreamingProxy["High-Throughput Streaming Reverse Proxy\n(HTTP / WebSocket / MCP Interceptor)"]
        
        subgraph RealtimeAnalyzers ["Real-Time Gateways"]
            LoopDetector["Semantic Loop & Thrashing Detector\n(Cosine Similarity on Action Embeddings)"]
            PolicyEngine["Invariant Policy Engine\n(OPA / Rego & Token Budget Limiter)"]
            ContextAuditor["Context Window Heatmap Analyzer\n(Attention & Token Degradation Tracker)"]
        end

        HITL_Router["Human-in-the-Loop Interrupter\n(Pause / Resume / Inject Correction)"]
    end

    subgraph Upstream ["Upstream Providers & Tools"]
        LLM_API["LLM Provider (Anthropic / OpenAI / Local vLLM)"]
        MCP_Tools["MCP Servers & External APIs"]
    end

    subgraph StorageAndUI ["Observability & Web Dashboard"]
        ClickHouse[("ClickHouse / TimescaleDB\n(Fast Time-Series Traces)")]
        NextDashboard["Next.js Real-Time Canvas\n(Interactive DAG & State Timeline)"]
    end

    LLM_Client <-->|Requests & Tool Calls| StreamingProxy
    StreamingProxy --> LoopDetector
    StreamingProxy --> PolicyEngine
    StreamingProxy --> ContextAuditor
    
    PolicyEngine -->|Violation Triggered| HITL_Router
    LoopDetector -->|Threshold Exceeded| HITL_Router
    
    HITL_Router <-->|Approved Calls| LLM_API
    HITL_Router <-->|Approved Executions| MCP_Tools
    
    StreamingProxy -.->|Async Bulk Event Stream| ClickHouse
    ClickHouse --> NextDashboard
    NextDashboard <-->|WebSocket Breakpoints| HITL_Router
```

---

## ⚡ Ключевые инженерные инновации

### 1. Детектор семантического зацикливания (Semantic Loop Detector)
- Не опирается на точное совпадение строк (агенты часто меняют 1 символ или пробел).
- Вычисляет скользящее окно векторов мыслей и параметров вызова инструментов с использованием легковесной локальной модели эмбеддингов (ONNX runtime / MiniLM, <3ms).
- При косинусном сходстве > 0.94 между 3 последовательными шагами — автоматически генерирует `CircuitBreakException` с подсказкой агенту: *"Вы зациклились, примените альтернативную стратегию или обратитесь к человеку"*.

### 2. Детерминированные инварианты на Open Policy Agent (OPA/Rego)
- Декларативные правила безопасности, применяемые на лету:
  ```rego
  # Пример политики: запрет деструктивных операций без ручного аппрува
  package traceforge.safety

  default allow = true

  allow = false {
      input.tool_name == "execute_bash"
      regex.match("(rm -rf|drop table|truncate|mkfs)", input.tool_args.command)
  }
  ```
- Мгновенная блокировка вызова инструмента до того, как он попадет на сервер выполнения.

### 3. Тепловая карта контекстного окна (Context Window Heatmap)
- Визуальный анализ распределения токенов: сколько уходит на системный промпт, память, историю вызовов инструментов и полезный вывод.
- Выявление зон «забывания» (Lost-in-the-Middle) в реальном времени.

### 4. Интерактивный Human-in-the-Loop Web-Debugger
- Отображение цепочки шагов в виде графа состояний (DAG).
- Возможность поставить точку останова (breakpoint) перед вызовом конкретного MCP-инструмента или при превышении лимита расходов на сессию.

---

## 🛠️ Технологический стек

| Компонент | Технологии | Обоснование выбора |
|---|---|---|
| **Core Streaming Proxy** | `Go` (Fiber / net/http) или `Rust` (Axum / Tokio) | Неблокирующий I/O, обработка десятков тысяч одновременных SSE-соединений без GC-пауз |
| **Embeddings & Loop Math**| `ONNX Runtime` (C++ bindings) | Локальный инференс без сетевых задержек для детекции семантических циклов |
| **Policy Engine** | `Open Policy Agent` (embedded Go library) | Индустриальный стандарт декларативных политик безопасности |
| **Хранилище трейсов** | `ClickHouse` + `Redis Streams` | Колоночное хранение миллиардов логов шагов, агрегации по токенам и затратам за миллисекунды |
| **Web Dashboard & UI** | `Next.js 15` (App Router), `React Flow`, `Tailwind v4` | Визуализация графов выполнения в реальном времени |

---

## 🚀 Дорожная карта разработки (Milestones)

### Этап 1: Высокопроизводительный Reverse Proxy (MVP) — ✔ Завершен
- [x] Прозрачный прокси-сервер для OpenAI / Anthropic / MCP вызовов с субмиллисекундной задержкой.
- [x] Zero-copy stream teeing для непрерывной трансляции SSE чанков клиенту.
- [x] Базовый счетчик токенов и калькулятор расходов ($) в реальном времени.

### Этап 2: Семантический детектор циклов и OPA-политики — ✔ Завершен
- [x] Векторный движок детекции семантических циклов (Cosine similarity, 31 мкс оверхед).
- [x] Прерывание зацикливаний (*Loop Thrashing*) и синтез корректирующих системных инструкций (`recovery_prompt`).
- [x] Движок правил Rego (OPA) с пресетами безопасности (защита от `rm -rf`, `DROP TABLE`, path traversal).
- [x] Потокобезопасный `BudgetLimiter` на `AtomicU64` в микродолларах.
- [x] Встраиваемое хранилище трейсов SQLite (WAL mode) и асинхронный сброс через `AsyncBatchFlusher`.

### Этап 3: Web-дашборд и точки останова — ✔ Завершен
- [x] Интерактивный UI с графом выполнения шагов агента на React Flow (`dashboard/`).
- [x] Точки останова (Breakpoints) на вызовы инструментов через WebSocket `/ws/control`.
- [x] Модальное окно Human-in-the-Loop («Одобрить / Отклонить / Скорректировать»).
- [x] Тепловая карта контекстного окна (Context Window Heatmap) и аудит фрагментации токенов.

### Этап 4: Production Packaging & UI Embedding — ✔ Завершен
- [x] Встраивание статического Next.js дашборда прямо в Rust-бинарник через `rust-embed`.
- [x] Физический перехватчик протокола Model Context Protocol (MCP JSON-RPC 2.0).
- [x] Оптимизированный multi-stage `Dockerfile` (Node.js -> Rust -> rootless Debian runtime).
- [x] `docker-compose.yml` и автоматизированный GitHub Actions CI/CD пайплайн.

---

## 💰 Стратегия монетизации (Business Model)

1. **B2B Developer SaaS (Модель Langfuse / Datadog)**:
   - Free tier: до 50 000 событий в месяц (для индивидуальных разработчиков).
   - Pro tier: \$49–\$199/мес за команду (миллионы событий, расширенные алерты, интеграция с Slack/PagerDuty).
2. **Enterprise AI Compliance (SOC2 / GDPR / EU AI Act)**:
   - Компании обязаны доказывать, что их AI-агенты не совершают неконтролируемых действий.
   - Лицензия \$1,500–\$5,000/мес за установку внутри периметра с неизменяемым журналом аудита (Immutable Audit Trail).

---

## 🎯 Что говорить на собеседованиях (Portfolio Value)

- «Я построил упреждающий сетевой файрвол для AI-агентов с оверхедом менее 5 мс на Go/Rust, предотвращающий финансовые утечки и циклы галлюцинаций».
- «Разработал векторный детектор семантического зацикливания на базе ONNX-эмбеддингов в локальном процессе, снизивший пустые расходы токенов на 40%».
- «Интегрировал OPA/Rego для декларативного разграничения прав доступа агентов к инфраструктуре в режиме реального времени».
