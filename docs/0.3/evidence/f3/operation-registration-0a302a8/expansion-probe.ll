; ModuleID = 'expansion_probe.f01821e47f8f796c-cgu.0'
source_filename = "expansion_probe.f01821e47f8f796c-cgu.0"
target datalayout = "e-m:e-p270:32:32-p271:32:32-p272:64:64-i64:64-i128:128-f80:128-n8:16:32:64-S128"
target triple = "x86_64-unknown-linux-gnu"

@alloc_e07dedd9abab2af4a5d1f1060953aefb = private unnamed_addr constant [20 x i8] c"configuration.create", align 1, !guid !0
@alloc_8dc90a64769074e078fc87e21ffada50 = private unnamed_addr constant [25 x i8] c"configuration.set_enabled", align 1, !guid !1

@manual = unnamed_addr alias { ptr, i64 } (ptr), ptr @generated

; Function Attrs: mustprogress nofree norecurse nosync nounwind nonlazybind willreturn memory(argmem: read) uwtable
define { ptr, i64 } @generated(ptr noalias nofree noundef readonly captures(none) dereferenceable(2) %command) unnamed_addr #0 !guid !6 {
start:
  %0 = load i8, ptr %command, align 1, !range !7, !noundef !8
  %1 = trunc nuw i8 %0 to i1
  %. = select i1 %1, i64 25, i64 20
  %alloc_8dc90a64769074e078fc87e21ffada50.alloc_e07dedd9abab2af4a5d1f1060953aefb = select i1 %1, ptr @alloc_8dc90a64769074e078fc87e21ffada50, ptr @alloc_e07dedd9abab2af4a5d1f1060953aefb
  %2 = insertvalue { ptr, i64 } poison, ptr %alloc_8dc90a64769074e078fc87e21ffada50.alloc_e07dedd9abab2af4a5d1f1060953aefb, 0
  %3 = insertvalue { ptr, i64 } %2, i64 %., 1
  ret { ptr, i64 } %3
}

attributes #0 = { mustprogress nofree norecurse nosync nounwind nonlazybind willreturn memory(argmem: read) uwtable "probe-stack"="inline-asm" "target-cpu"="x86-64" }

!llvm.module.flags = !{!2, !3, !4}
!llvm.ident = !{!5}

!0 = !{i64 1851073812903130498}
!1 = !{i64 -2981711689521000672}
!2 = !{i32 8, !"PIC Level", i32 2}
!3 = !{i32 2, !"RtLibUseGOT", i32 1}
!4 = !{i32 7, !"uwtable", i32 2}
!5 = !{!"rustc version 1.100.0 (0ef1bc386 2026-11-09)"}
!6 = !{i64 4419172327205881924}
!7 = !{i8 0, i8 2}
!8 = !{}
