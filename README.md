# ⚡ TraceForge (AgentOps Guard)

> **Real-Time Invariant Firewall, Loop-Interception Proxy & Time-Travel Debugger for Autonomous LLM Agents in Production.**

[![Proxy Latency](https://img.shields.io/badge/Proxy%20Latency-<5ms-brightgreen.svg)](#proxy-architecture)
[![Policy Engine](https://img.shields.io/badge/Guardrails-OPA%20%2F%20Rego-blue.svg)](#policy--invariant-engine)
[![Observability](https://img.shields.io/badge/Telemetry-OpenTelemetry%20%2B%20ClickHouse-orange.svg)](#storage--analytics)
[![License](https://img.shields.io/badge/License-Dual%20AGPLv3%20%2F%20Enterprise-blueviolet.svg)](#monetization--business-model)

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

### Этап 1: Высокопроизводительный Reverse Proxy (MVP, 2 недели)
- [ ] Прозрачный прокси-сервер для OpenAI / Anthropic / MCP вызовов с замером задержки.
- [ ] Базовый счетчик токенов и калькулятор расходов ($) в реальном времени.
- [ ] Простой детектор точных повторов вызовов тулов.

### Этап 2: Семантический детектор циклов и OPA-политики (3-4 недели)
- [ ] Интеграция ONNX runtime для эмбеддингов действий агента.
- [ ] Движок правил Rego с пресетами безопасности (защита файлов, БД, сети).
- [ ] Механизм впрыска корректирующих системных инструкций (System Prompt Injection).

### Этап 3: Web-дашборд и точки останова (5-8 недель)
- [ ] Интерактивный UI с графом выполнения шагов (React Flow).
- [ ] Точки останова (Breakpoints) и кнопка «Одобрить / Отклонить действие» через WebSocket.
- [ ] Экспорт трейсов в OpenTelemetry-совместимый формат.

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
