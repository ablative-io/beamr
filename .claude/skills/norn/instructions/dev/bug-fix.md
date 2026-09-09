
Reproduce or otherwise prove the defect before changing production behavior.
Find the earliest incorrect boundary, implement the smallest complete repair,
and add a regression that fails for the original reason. Check adjacent paths
for the same mechanism and verify both failure and success behavior.
