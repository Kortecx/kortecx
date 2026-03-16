from __future__ import annotations

import logging
from dataclasses import dataclass, field
from enum import Enum
from typing import Any

logger = logging.getLogger("engine.training")


class TrainingMethod(str, Enum):
    SFT = "sft"
    RLHF = "rlhf"
    DPO = "dpo"
    ORPO = "orpo"


@dataclass
class TrainingConfig:
    model_id: str
    dataset_id: str
    method: TrainingMethod = TrainingMethod.SFT
    output_dir: str = "./outputs"
    epochs: int = 3
    batch_size: int = 4
    learning_rate: float = 2e-5
    max_seq_length: int = 2048
    lora_r: int = 16
    lora_alpha: int = 32
    lora_dropout: float = 0.05
    use_unsloth: bool = True
    extra: dict[str, Any] = field(default_factory=dict)


class TrainingService:
    """Model fine-tuning via TRL, Unsloth, SFT, RLHF."""

    def sft_train(self, config: TrainingConfig) -> dict[str, Any]:
        """Supervised fine-tuning with optional Unsloth acceleration."""
        logger.info("Starting SFT: model=%s dataset=%s unsloth=%s", config.model_id, config.dataset_id, config.use_unsloth)

        if config.use_unsloth:
            return self._sft_unsloth(config)
        return self._sft_trl(config)

    def _sft_unsloth(self, config: TrainingConfig) -> dict[str, Any]:
        from unsloth import FastLanguageModel

        model, tokenizer = FastLanguageModel.from_pretrained(
            model_name=config.model_id,
            max_seq_length=config.max_seq_length,
            load_in_4bit=True,
        )

        model = FastLanguageModel.get_peft_model(
            model,
            r=config.lora_r,
            lora_alpha=config.lora_alpha,
            lora_dropout=config.lora_dropout,
            target_modules=["q_proj", "k_proj", "v_proj", "o_proj", "gate_proj", "up_proj", "down_proj"],
        )

        from datasets import load_dataset
        from trl import SFTTrainer, SFTConfig

        dataset = load_dataset(config.dataset_id, split="train")

        trainer = SFTTrainer(
            model=model,
            tokenizer=tokenizer,
            train_dataset=dataset,
            args=SFTConfig(
                output_dir=config.output_dir,
                num_train_epochs=config.epochs,
                per_device_train_batch_size=config.batch_size,
                learning_rate=config.learning_rate,
                logging_steps=10,
                save_strategy="epoch",
            ),
        )

        result = trainer.train()
        trainer.save_model(config.output_dir)

        return {
            "method": "sft_unsloth",
            "model": config.model_id,
            "dataset": config.dataset_id,
            "loss": result.training_loss,
            "epochs": config.epochs,
            "output_dir": config.output_dir,
        }

    def _sft_trl(self, config: TrainingConfig) -> dict[str, Any]:
        from transformers import AutoModelForCausalLM, AutoTokenizer
        from peft import LoraConfig
        from trl import SFTTrainer, SFTConfig
        from datasets import load_dataset

        tokenizer = AutoTokenizer.from_pretrained(config.model_id)
        model = AutoModelForCausalLM.from_pretrained(config.model_id, device_map="auto")

        peft_config = LoraConfig(
            r=config.lora_r,
            lora_alpha=config.lora_alpha,
            lora_dropout=config.lora_dropout,
            target_modules=["q_proj", "k_proj", "v_proj", "o_proj"],
        )

        dataset = load_dataset(config.dataset_id, split="train")

        trainer = SFTTrainer(
            model=model,
            tokenizer=tokenizer,
            train_dataset=dataset,
            peft_config=peft_config,
            args=SFTConfig(
                output_dir=config.output_dir,
                num_train_epochs=config.epochs,
                per_device_train_batch_size=config.batch_size,
                learning_rate=config.learning_rate,
                logging_steps=10,
                save_strategy="epoch",
            ),
        )

        result = trainer.train()
        trainer.save_model(config.output_dir)

        return {
            "method": "sft_trl",
            "model": config.model_id,
            "dataset": config.dataset_id,
            "loss": result.training_loss,
            "epochs": config.epochs,
            "output_dir": config.output_dir,
        }

    def dpo_train(self, config: TrainingConfig) -> dict[str, Any]:
        """Direct Preference Optimization training."""
        from transformers import AutoModelForCausalLM, AutoTokenizer
        from trl import DPOTrainer, DPOConfig
        from datasets import load_dataset
        from peft import LoraConfig

        logger.info("Starting DPO: model=%s dataset=%s", config.model_id, config.dataset_id)

        tokenizer = AutoTokenizer.from_pretrained(config.model_id)
        model = AutoModelForCausalLM.from_pretrained(config.model_id, device_map="auto")

        peft_config = LoraConfig(
            r=config.lora_r,
            lora_alpha=config.lora_alpha,
            lora_dropout=config.lora_dropout,
        )

        dataset = load_dataset(config.dataset_id, split="train")

        trainer = DPOTrainer(
            model=model,
            tokenizer=tokenizer,
            train_dataset=dataset,
            peft_config=peft_config,
            args=DPOConfig(
                output_dir=config.output_dir,
                num_train_epochs=config.epochs,
                per_device_train_batch_size=config.batch_size,
                learning_rate=config.learning_rate,
                logging_steps=10,
            ),
        )

        result = trainer.train()
        trainer.save_model(config.output_dir)

        return {
            "method": "dpo",
            "model": config.model_id,
            "dataset": config.dataset_id,
            "loss": result.training_loss,
            "output_dir": config.output_dir,
        }


training_service = TrainingService()
