# midx

**`midx`** é uma ferramenta CLI local (*offline-first*) de alta performance escrita em Rust para indexação e busca semântica em repositórios de notas Markdown. Ela combina parser AST de Markdown, embeddings multilíngues locais via ONNX (suportando português e termos técnicos em inglês) e persistência transacional em SQLite.

---

## Sumário

- [Instalação](#instalação)
- [Guia Rápido de Uso](#guia-rápido-de-uso)
- [Comandos da CLI](#comandos-da-cli)
- [Configuração (`.midx.toml`)](#configuração-midxtoml)
- [Uso Offline e Cache do Modelo](#uso-offline-e-cache-do-modelo)
- [Como Funciona por Dentro](#como-funciona-por-dentro)
  - [1. Arquitetura Hexagonal](#1-arquitetura-hexagonal)
  - [2. Descoberta e Detecção Incremental de Mudanças](#2-descoberta-e-detecção-incremental-de-mudanças)
  - [3. Chunking Semântico via AST Markdown](#3-chunking-semântico-via-ast-markdown)
  - [4. Pipeline em Lote (*Pipes & Filters*) e Embeddings E5](#4-pipeline-em-lote-pipes--filters-e-embeddings-e5)
  - [5. Persistência Atômica no SQLite (*Unit of Work*)](#5-persistência-atômica-no-sqlite-unit-of-work)
  - [6. Busca Semântica e Ranking Top-K](#6-busca-semântica-e-ranking-top-k)
- [Desenvolvimento e Testes](#desenvolvimento-e-testes)

---

## Instalação

Pré-requisito: toolchain Rust (Cargo). Não é necessário instalar OpenSSL nem SQLite no sistema (`rusqlite` usa `bundled` e o cliente HTTP usa `rustls`).

```bash
# Compilar em modo release
cargo build --release

# Ou instalar globalmente em ~/.cargo/bin
cargo install --path .
```

---

## Guia Rápido de Uso

```bash
# 1. Entre na pasta das suas notas e inicialize o repositório
cd ~/minhas-notas
midx init

# 2. Indexe todas as notas Markdown (no 1º uso, o modelo ONNX será baixado para cache local)
midx sync

# 3. Faça buscas semânticas por significado, mesmo misturando idiomas
midx search "como garantir atomicidade ao atualizar o banco"

# 4. Veja estatísticas do índice
midx status
```

Você pode executar `sync`, `search` e `status` de qualquer **subdiretório** dentro do repositório de notas — o `midx` sobe na árvore de diretórios até encontrar o `.midx.toml` mais próximo (de forma semelhante ao `.git`).

---

## Comandos da CLI

### `midx init [PATH]`

Inicializa um repositório no diretório `PATH` (padrão: `.`):
- Cria `.midx.toml` com a configuração padrão (caso ainda não exista).
- Cria o diretório `.midx/` e inicializa o banco `.midx/index.db` com o schema e pragmas do SQLite.
- Adiciona `.midx/` ao `.gitignore` local (se ainda não estiver listado).

### `midx sync [--force]`

Executa a sincronização incremental entre o disco e o banco:
- Varre arquivos `.md` recursivamente, ignorando `.git/` e quaisquer arquivos/diretórios ocultos (nomes iniciados com `.`, incluindo `.midx/`).
- Pula arquivos cujo conteúdo não mudou (comparando `mtime` e `Sha256Hash`).
- Remove do banco arquivos deletados do disco (com remoção em cascata dos seus chunks).
- `--force`: força a reindexação e revetorização de todos os arquivos `.md`, mesmo sem alterações.

Exemplo de saída:

```text
Sync completed
  added:     12
  updated:   2
  removed:   1
  unchanged: 48
  chunks:    67
  elapsed:   1.42s
```

### `midx search <QUERY> [-n <LIMIT>] [--no-sync]`

Executa uma busca semântica por similaridade de cosseno:
- Por padrão, roda um `sync` incremental rápido antes de buscar para garantir resultados atualizados.
- `--no-sync`: pula o `sync` prévio para buscas instantâneas quando o índice já está atualizado.
- `-n <LIMIT>` / `--limit <LIMIT>`: define quantos trechos retornar (padrão: `5`).

Cada resultado exibe o rank, o score de similaridade, o caminho relativo com a linha inicial (`arquivo.md:linha`), a trilha hierárquica de títulos e um trecho do conteúdo.

### `midx status`

Exibe um resumo do índice sem carregar o modelo ONNX:
- Diretório raiz encontrado e caminho do banco SQLite.
- Modelo configurado e `batch_size`.
- Total de arquivos indexados, total de chunks e tamanho do arquivo `.db` em disco.

---

## Configuração (`.midx.toml`)

Arquivo gerado por `midx init`:

```toml
model = "MultilingualE5Small"
batch_size = 32
max_chunk_words = 480
db_path = ".midx/index.db"
```

| Chave | Padrão | Descrição |
| --- | --- | --- |
| `model` | `"MultilingualE5Small"` | Modelo de embedding (`MultilingualE5Small`, `MultilingualE5Base` ou `MultilingualE5Large`). |
| `batch_size` | `32` | Quantidade de chunks agrupados por chamada ao motor ONNX durante o `sync`. |
| `max_chunk_words` | `480` | Orçamento aproximado de palavras por chunk semântico antes de dividir seções longas. |
| `db_path` | `".midx/index.db"` | Caminho do banco SQLite (relativo à raiz onde está o `.midx.toml` ou absoluto). |

---

## Uso Offline e Cache do Modelo

Na primeira execução de `midx sync` ou `midx search`, a biblioteca `fastembed` baixa os arquivos do modelo `intfloat/multilingual-e5-small` (384 dimensões) via `hf-hub` e os armazena no cache local (configurável via `FASTEMBED_CACHE_DIR` ou `HF_HOME`). Depois disso, todas as operações de indexação e busca rodam **100% offline**, localmente na CPU.

---

## Como Funciona por Dentro

### 1. Arquitetura Hexagonal

O projeto separa estritamente regras de negócio, orquestração e adaptadores externos:

```text
src/
├── main.rs                  # Composition Root (instancia adaptadores concretos) e error boundary (anyhow)
├── cli.rs                   # Driving Adapter (subcomandos e flags via clap)
├── config.rs                # Descoberta recursiva de .midx.toml, defaults e .gitignore
├── domain/
│   ├── models.rs            # Newtypes (Sha256Hash, Embedding) e entidades puras
│   ├── errors.rs            # MidxError tipado com thiserror
│   ├── chunker.rs           # Parser AST Markdown puro com pulldown-cmark (zero I/O)
│   └── similarity.rs        # Similaridade de cosseno (f64) e ordenação Top-K
├── application/
│   ├── ports.rs             # Traits: FileScanner, TextEmbedder, NoteRepository
│   ├── sync.rs              # SyncUseCase: diff incremental + batching + Unit of Work
│   └── search.rs            # SearchUseCase: sync opcional + embedding da query + Top-K
└── infrastructure/
    ├── fs_scanner.rs        # FileScanner concreto (walkdir + sha2)
    ├── onnx_embedder.rs     # TextEmbedder concreto (fastembed / ONNX Runtime)
    └── sqlite_repo.rs       # NoteRepository concreto (rusqlite em modo WAL)
```

As camadas `domain` e `application` **não importam** `rusqlite`, `fastembed`, `walkdir` ou `sha2`; toda comunicação externa passa pelas traits definidas em `application/ports.rs`. Isso permite testar todo o pipeline incremental com mocks em memória, sem disco nem ONNX.

### 2. Descoberta e Detecção Incremental de Mudanças

1. `FsScanner::scan` percorre a árvore com `walkdir` (`follow_links(false)`), filtrando qualquer entrada cujo nome comece com `.` (como `.git/`, `.midx/`, `.obsidian/`) e selecionando apenas arquivos `.md`.
2. Para cada arquivo, lê `mtime` (em milissegundos desde `UNIX_EPOCH`) e calcula o digest SHA-256 em streaming (buffer de 8 KiB), encapsulado no newtype `Sha256Hash`.
3. `SyncUseCase` compara o conjunto escaneado com a tabela `files` no banco:
   - Ausente no banco → **adicionado**.
   - Presente no banco com `hash` igual e `--force` desativado → **inalterado** (pulado sem revetorizar).
   - `hash` diferente ou `--force` ativo → **atualizado**.
   - Presente no banco mas ausente no disco → **removido** via `delete_files`.

### 3. Chunking Semântico via AST Markdown

O `MarkdownChunker` (`domain/chunker.rs`) usa `pulldown-cmark` com `into_offset_iter()` e extensões habilitadas (tabelas, notas de rodapé, tachado e tasklists):

- **Hierarquia de cabeçalhos:** mantém uma pilha de títulos ativa (`# H1 > ## H2 > ### H3`). Ao encontrar um título de nível menor ou igual ao topo da pilha, desempilha os anteriores e atualiza a trilha (`heading_context`).
- **Precisão de linha:** pré-computa os offsets de quebra de linha do texto e converte o byte offset de cada evento AST em número de linha 1-based (`line_start`) via busca binária.
- **Proteção de blocos de código:** por usar AST real, linhas como `# comentário` dentro de blocos ``` ``` ``` não são confundidas com cabeçalhos Markdown.
- **Divisão controlada:** acumula blocos lógicos consecutivos dentro da mesma seção até o limite de `max_chunk_words` (~480 palavras). Seções maiores (ou blocos únicos gigantes) são quebradas mantendo o mesmo `heading_context` e recalculando o `line_start` de cada parte.

### 4. Pipeline em Lote (*Pipes & Filters*) e Embeddings E5

Em vez de vetorizar um chunk por vez, o `SyncUseCase` enfileira os chunks extraídos de arquivos novos/editados em lotes de tamanho `batch_size` (padrão `32`):

1. Para cada chunk, monta o texto de passagem combinando `heading_context` + `content` (para que títulos também contribuam para o significado semântico).
2. Ao atingir `batch_size` (ou ao final da fila), chama `TextEmbedder::embed_passages` uma única vez para todo o lote.
3. No `OnnxEmbedder`, os prefixos exigidos pela família **E5** são aplicados automaticamente:
   - `"passage: "` em cada chunk indexado durante o `sync`.
   - `"query: "` na consulta durante o `search`.

### 5. Persistência Atômica no SQLite (*Unit of Work*)

O `SqliteStore` configura o banco na abertura com:

```sql
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
```

Schema:

```sql
CREATE TABLE IF NOT EXISTS files (
    path TEXT PRIMARY KEY,
    hash TEXT NOT NULL,
    mtime INTEGER NOT NULL,
    indexed_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS chunks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    file_path TEXT NOT NULL REFERENCES files(path) ON DELETE CASCADE,
    heading_context TEXT NOT NULL,
    content TEXT NOT NULL,
    line_start INTEGER NOT NULL,
    embedding BLOB NOT NULL
);
```

- **Serialização binária:** `Embedding` serializa `Vec<f32>` diretamente para bytes *little-endian* (`to_le_bytes` / `from_le_bytes`) com validação de múltiplo de 4 bytes.
- **Integridade contra `SIGINT` (`Ctrl+C`):** assim que todos os chunks de um arquivo terminam de ser vetorizados nos lotes, `save_file_chunks` executa uma única transação SQLite (`DELETE` dos chunks antigos + `UPSERT` em `files` + `INSERT` dos novos chunks + `COMMIT`). Se o processo for interrompido no meio, nenhum arquivo fica parcialmente indexado.
- **Deleção em cascata:** remover um registro de `files` apaga automaticamente todos os seus `chunks` via `ON DELETE CASCADE`.

### 6. Busca Semântica e Ranking Top-K

No `SearchUseCase`:
1. Opcionalmente executa um `sync` incremental (a menos que `--no-sync` seja passado).
2. Gera o vetor da query com prefixo `"query: "`.
3. Carrega os `StoredChunk` do banco e calcula a similaridade de cosseno em precisão `f64` (`domain/similarity.rs`), retornando `0.0` de forma segura caso haja vetor nulo ou dimensão incompatível.
4. Ordena por score decrescente com `f32::total_cmp` (sem risco de pânico com `NaN`) e trunca nos `limit` melhores resultados.

---

## Desenvolvimento e Testes

```bash
# Rodar linter estrito
cargo clippy --all-targets -- -D warnings

# Rodar testes unitários (puros, sem ONNX ou disco)
cargo test
```

Os testes unitários cobrem:
- `domain::chunker::tests`: extração de hierarquia de títulos, `line_start`, código com `#`, preâmbulo e quebra de seções longas.
- `domain::similarity::tests`: similaridade de cosseno, roundtrip *little-endian* de `Embedding` e ordenação `top_k`.
- `application::sync::tests`: fluxo completo com `MockScanner`, `MockEmbedder` e `InMemoryRepo` testando arquivos novos, inalterados, editados, removidos, `--force` e agrupamento em lotes.
