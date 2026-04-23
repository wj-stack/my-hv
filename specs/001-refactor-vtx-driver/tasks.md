# Tasks: Rust 重构 VT-x 虚拟化驱动

**Input**: Design documents from `/specs/001-refactor-vtx-driver/`  
**Prerequisites**: plan.md (required), spec.md (required), research.md, data-model.md, contracts/, quickstart.md

**Tests**: 未要求 TDD；本任务清单不包含测试实现步骤，仅保留验收/验证动作。

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Project initialization and basic structure

- [ ] T001 Create module layout in `driver/src/` (`vmx.rs`, `vmcs.rs`, `vcpu.rs`, `exit_handlers.rs`, `ept.rs`, `hypercalls.rs`, `introspection.rs`, `logger.rs`, `mm.rs`, `timing.rs`, `arch.rs`, `gdt.rs`, `idt.rs`, `segment.rs`) and wire in `driver/src/lib.rs`
- [ ] T002 [P] Add mapping notes in the new modules referencing counterparts in `hv/hv/*`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core infrastructure that MUST be complete before ANY user story can be implemented

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [ ] T003 Define hypercall key, contract version, and hypercall codes in `shared-contract/src/lib.rs`
- [ ] T004 Implement driver entry/unload flow in `driver/src/lib.rs` aligned with `hv/hv/main.cpp` (start/stop virtualization + ping)
- [ ] T005 Implement VMCALL dispatch skeleton in `driver/src/hypercalls.rs` returning explicit errors for unimplemented ops
- [ ] T006 Update `client/src/main.rs` or `hv/um/*` to add hypercall commands with placeholders

**Checkpoint**: Foundation ready - user story implementation can now begin in parallel

---

## Phase 3: User Story 1 - 维持核心虚拟化能力 (Priority: P1) 🎯 MVP

**Goal**: 在支持 VT-x 的硬件上完成虚拟化初始化与运行的核心闭环。

**Independent Test**: 通过用户态工具触发 `PING`/`UNLOAD`，确保驱动进入运行状态并可正常退出。

### Implementation for User Story 1

- [ ] T007 [P] [US1] Implement VT-x capability checks and VMXON/VMXOFF flow in `driver/src/vmx.rs`
- [ ] T008 [P] [US1] Implement VCPU/VMCS initialization lifecycle in `driver/src/vcpu.rs` and `driver/src/vmcs.rs`
- [ ] T009 [US1] Implement `PING`/`UNLOAD` handlers in `driver/src/hypercalls.rs` (depends on T007, T008)
- [ ] T010 [US1] Wire VMCALL handling into VM-exit path in `driver/src/exit_handlers.rs` (depends on T009)
- [ ] T011 [US1] Implement `PING`/`UNLOAD` calls in `client/src/main.rs` or `hv/um/*`

**Checkpoint**: At this point, User Story 1 should be fully functional and testable independently

---

## Phase 4: User Story 2 - 新接口与用户态工具 (Priority: P2)

**Goal**: 交付新的 VMCALL 契约与用户态工具，使新接口可被完整使用。

**Independent Test**: 用户态工具可调用查询类 hypercall 并输出状态或日志摘要。

### Implementation for User Story 2

- [ ] T012 [US2] Implement `QUERY_PROCESS_CR3`/`GET_HV_BASE`/`FLUSH_LOGS` handlers in `driver/src/hypercalls.rs`
- [ ] T013 [US2] Add query/log commands in `client/src/main.rs` or `hv/um/*`
- [ ] T014 [US2] Sync hypercall contract description in `specs/001-refactor-vtx-driver/contracts/driver-interface.md`

**Checkpoint**: At this point, User Stories 1 AND 2 should both work independently

---

## Phase 5: User Story 3 - 稳定性与可维护性提升 (Priority: P3)

**Goal**: 异常输入、卸载边界与故障诊断更清晰，降低维护成本。

**Independent Test**: 非法参数、VT-x 不支持、运行中卸载等场景返回可诊断错误且无系统崩溃。

### Implementation for User Story 3

- [ ] T015 [P] [US3] Add hypercall argument validation and error mapping in `driver/src/hypercalls.rs`
- [ ] T016 [P] [US3] Add safe shutdown path for active VCPUs in `driver/src/lib.rs`
- [ ] T017 [US3] Add diagnostic logging in `driver/src/logger.rs` and wire in `driver/src/exit_handlers.rs`

**Checkpoint**: All user stories should now be independently functional

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Improvements that affect multiple user stories

- [ ] T018 [P] Update `README.md` with new hypercall command examples
- [ ] T019 Update `specs/001-refactor-vtx-driver/quickstart.md` with validated steps and any caveats
- [ ] T020 [P] Run `build.bat` and record a short build/validation note in `specs/001-refactor-vtx-driver/quickstart.md`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies - can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion - BLOCKS all user stories
- **User Stories (Phase 3+)**: All depend on Foundational phase completion
  - User stories can proceed in parallel (if staffed)
  - Or sequentially in priority order (P1 → P2 → P3)
- **Polish (Final Phase)**: Depends on all desired user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Can start after Foundational (Phase 2) - No dependencies on other stories
- **User Story 2 (P2)**: Can start after Foundational (Phase 2) - Builds on US1 but should remain independently testable
- **User Story 3 (P3)**: Can start after Foundational (Phase 2) - Enhances stability for US1/US2

### Within Each User Story

- Contracts and shared constants before driver/user-mode wiring
- Driver state and handlers before user-mode verification
- Core implementation before validation steps

### Parallel Opportunities

- Phase 1 tasks marked [P] can run in parallel
- In Phase 3, `driver/src/vmx.rs` and `driver/src/vcpu.rs` can be implemented in parallel
- US2 and US3 tasks can run in parallel once US1 core is stable

---

## Parallel Example: User Story 1

```bash
Task: "Implement VT-x capability checks in driver/src/vmx.rs"
Task: "Implement VCPU/VMCS initialization in driver/src/vcpu.rs and driver/src/vmcs.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational (CRITICAL - blocks all stories)
3. Complete Phase 3: User Story 1
4. **STOP and VALIDATE**: Test User Story 1 independently
5. Deploy/demo if ready

### Incremental Delivery

1. Complete Setup + Foundational → Foundation ready
2. Add User Story 1 → Test independently → Deploy/Demo (MVP)
3. Add User Story 2 → Test independently → Deploy/Demo
4. Add User Story 3 → Validate stability improvements
5. Each story adds value without breaking previous stories

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together
2. Once Foundational is done:
   - Developer A: User Story 1
   - Developer B: User Story 2
   - Developer C: User Story 3
3. Stories complete and integrate independently
