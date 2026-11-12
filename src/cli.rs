use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Local offline-first semantic indexer and search CLI for Markdown repositories.
#[derive(Debug, Parser)]
#[command(
    name = "midx",
    version,
    about = "Indexador e buscador semantico local (offline-first) para notas em Markdown"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Inicializa um repositorio midx (.midx.toml, .midx/index.db e .gitignore)
    Init {
        /// Diretorio alvo onde o repositorio sera inicializado
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// Sincroniza arquivos Markdown modificados, novos ou removidos com o indice
    Sync {
        /// Forca a reindexacao de todos os arquivos mesmo sem alteracoes
        #[arg(long)]
        force: bool,
    },

    /// Busca trechos semanticamente similares a consulta fornecida
    Search {
        /// Texto da busca semantica
        query: String,

        /// Quantidade maxima de resultados exibidos
        #[arg(short = 'n', long, default_value_t = 5)]
        limit: usize,

        /// Pula a sincronizacao incremental antes da busca
        #[arg(long)]
        no_sync: bool,
    },

    /// Exibe estatisticas do indice e modelo configurado
    Status,
}

